use crate::{Artwork, Cache, CacheLimits, Error, Image, ImageSpec, Result, decode, providers::{Client, Http, Transport}, secrets::Secrets};
use std::{path::PathBuf, sync::{Arc, Mutex, mpsc::{self, Receiver, SyncSender, TrySendError}}, thread};

pub struct Job { pub key: String, pub artwork: Vec<Artwork>, pub spec: Option<ImageSpec> }
pub struct Loaded { pub path: Option<PathBuf>, pub image: Option<Image>, pub credit: String }
pub struct Done { pub key: String, pub result: Result<Loaded> }
/// Queue and completed-image admission are bounded together; an idle UI cannot accumulate decoded RAM.
pub struct Loader { jobs: Option<SyncSender<Job>>, done: Receiver<Done>, inflight: usize }
const MAX_INFLIGHT: usize = 16;
impl Loader {
    pub fn new(root: PathBuf, limits: CacheLimits, workers: usize) -> Result<Self> {
        Self::with_client(root, limits, workers, Client::<Http>::new(Secrets::default()))
    }
    pub fn with_client<T: Transport>(root: PathBuf, limits: CacheLimits, workers: usize, client: Client<T>) -> Result<Self> {
        // Storage failure must not prevent downloaded artwork from displaying.
        let cache = Arc::new(Mutex::new(Cache::open(root, limits).ok()));
        let client = Arc::new(client);
        let (jobs, incoming) = mpsc::sync_channel::<Job>(MAX_INFLIGHT);
        let (finished, done) = mpsc::sync_channel(MAX_INFLIGHT);
        let incoming = Arc::new(Mutex::new(incoming));
        for number in 0..workers.clamp(1, 2) {
            let (incoming, finished, cache, client) = (incoming.clone(), finished.clone(), cache.clone(), client.clone());
            thread::Builder::new().name(format!("lumagame-art-{number}")).spawn(move || {
                loop {
                    let job = match incoming.lock() { Ok(rx) => match rx.recv() { Ok(job) => job, Err(_) => break }, Err(_) => break };
                    let result = load(&client, &cache, &job, limits.image_bytes);
                    if finished.send(Done { key: job.key, result }).is_err() { break; }
                }
            })?;
        }
        Ok(Self { jobs: Some(jobs), done, inflight: 0 })
    }

    /// Never waits for network, disk, another worker or a free queue slot.
    pub fn request(&mut self, job: Job) -> Result<bool> {
        if self.inflight >= MAX_INFLIGHT { return Ok(false); }
        if job.artwork.len() > 32 || job.key.len() > 4096 || job.artwork.iter().any(|a| a.url.len() > 8192) {
            return Err(Error::Invalid("artwork request exceeds limits"));
        }
        let jobs = self.jobs.as_ref().ok_or(Error::WorkerClosed)?;
        match jobs.try_send(job) {
            Ok(()) => { self.inflight += 1; Ok(true) },
            Err(TrySendError::Full(_)) => Ok(false),
            Err(TrySendError::Disconnected(_)) => Err(Error::WorkerClosed),
        }
    }
    pub fn poll(&mut self) -> Option<Done> {
        let done = self.done.try_recv().ok()?;
        self.inflight -= 1;
        Some(done)
    }
    pub fn inflight(&self) -> usize { self.inflight }
}
impl Drop for Loader { fn drop(&mut self) { self.jobs = None; } }

fn load<T: Transport>(client: &Client<T>, cache: &Mutex<Option<Cache>>, job: &Job, image_limit: usize) -> Result<Loaded> {
    let mut last_error = Error::Invalid("no artwork available; use the title plate");
    for artwork in &job.artwork {
        let cached = cache.lock().ok().and_then(|mut cache| cache.as_mut().and_then(|c| c.get(&artwork.url).ok().flatten()));
        if let Some((path, bytes)) = cached {
            match pixels(&bytes, job.spec) {
                Ok(image) => return Ok(Loaded { path: Some(path), image, credit: artwork.credit.clone() }),
                Err(_) => { if let Ok(mut cache) = cache.lock() { if let Some(cache) = cache.as_mut() { let _ = cache.remove(&artwork.url); } } }
            }
        }
        match client.image(&artwork.url).and_then(|bytes| {
            if bytes.len() > image_limit { return Err(Error::Invalid("artwork exceeds cache image limit")); }
            let image = pixels(&bytes, job.spec)?;
            let path = cache.lock().ok().and_then(|mut cache| cache.as_mut().and_then(|c| c.put(&artwork.url, &bytes).ok().flatten()));
            Ok(Loaded { path, image, credit: artwork.credit.clone() })
        }) {
            Ok(loaded) => return Ok(loaded),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}
fn pixels(bytes: &[u8], spec: Option<ImageSpec>) -> Result<Option<Image>> {
    let source = decode::read(bytes)?;
    spec.map(|spec| decode::normalize(source, spec)).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, providers::{Request, Response}};
    use std::{collections::VecDeque, io::Cursor, time::Duration};
    struct Replay(Mutex<VecDeque<Response>>);
    impl Transport for Replay {
        fn execute(&self, _: Request) -> Result<Response> {
            Ok(self.0.lock().unwrap().pop_front().expect("unexpected network request"))
        }
    }
    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([12, 34, 56])))
            .write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
    fn artwork(url: &str) -> Artwork { Artwork { kind: Kind::Cover, url: url.into(), credit: "Fixture".into() } }
    #[test]
    fn workers_fall_through_missing_art_and_reuse_validated_disk_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let transport = Replay(Mutex::new(VecDeque::from([
            Response::new(404, vec![]), Response::new(200, png()),
        ])));
        let client = Client::with_transport(transport, Secrets::default());
        let mut loader = Loader::with_client(dir.path().join("art"), CacheLimits::default(), 2, client).unwrap();
        let job = || Job { key: "fixture".into(), artwork: vec![artwork("https://fixture/absent"), artwork("https://fixture/cover")], spec: Some(ImageSpec { width: 2, height: 2, contain: false, trim_alpha: false }) };
        assert!(loader.request(job()).unwrap());
        let done = loader.done.recv_timeout(Duration::from_millis(500)).unwrap();
        let loaded = done.result.unwrap();
        let path = loaded.path.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), png());
        assert_eq!(loaded.image.unwrap().rgba, [12, 34, 56, 255].repeat(4));
        // Read the successful source directly: no failed-source retry obscures the cache-hit assertion.
        let cache = Mutex::new(Some(Cache::open(dir.path().join("art"), CacheLimits::default()).unwrap()));
        let client = Client::with_transport(Replay(Mutex::new(VecDeque::new())), Secrets::default());
        let loaded = load(&client, &cache, &Job { key: "cached".into(), artwork: vec![artwork("https://fixture/cover")], spec: None }, 16 << 20).unwrap();
        assert_eq!(loaded.path.unwrap(), path);
        assert!(loaded.image.is_none());
    }
    #[test]
    fn corrupt_disk_entry_refetches_and_failed_storage_still_returns_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = Cache::open(dir.path().join("art"), CacheLimits::default()).unwrap();
        cache.put("https://fixture/cover", b"corrupt").unwrap();
        let cache = Mutex::new(Some(cache));
        let client = Client::with_transport(Replay(Mutex::new(VecDeque::from([Response::new(200, png())]))), Secrets::default());
        let job = Job { key: "a".into(), artwork: vec![artwork("https://fixture/cover")], spec: Some(ImageSpec { width: 1, height: 1, contain: false, trim_alpha: false }) };
        assert!(load(&client, &cache, &job, 16 << 20).unwrap().image.is_some());
        let client = Client::with_transport(Replay(Mutex::new(VecDeque::from([Response::new(200, png())]))), Secrets::default());
        let loaded = load(&client, &Mutex::new(None), &job, 16 << 20).unwrap();
        assert!(loaded.path.is_none());
        assert!(loaded.image.is_some());
    }
    #[test]
    fn admission_stays_bounded_when_ui_does_not_drain_results() {
        let (jobs, _receiver) = mpsc::sync_channel(MAX_INFLIGHT);
        let (_finished, done) = mpsc::sync_channel(MAX_INFLIGHT);
        let mut loader = Loader { jobs: Some(jobs), done, inflight: 0 };
        for _ in 0..MAX_INFLIGHT {
            assert!(loader.request(Job { key: "fixture".into(), artwork: vec![], spec: None }).unwrap());
        }
        assert!(!loader.request(Job { key: "overflow".into(), artwork: vec![], spec: None }).unwrap());
        assert_eq!(loader.inflight(), MAX_INFLIGHT);
    }
}
