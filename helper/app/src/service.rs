//! The service: starts every component, wires them together, and stops them in order.
//!
//! Components are threads that take a [`Ctx`](crate::ctx::Ctx): the screen watcher, the receiver client, the weather and news
//! runner, and the HTTP server. They talk through the hub (state out) and one command channel each (wake-ups in), and they all
//! end when the shared [`Stop`] is requested.
//!
//! Seam for later phases: the MCP server (phase 3) and the plugin broker (phase 9) are two more entries in
//! [`Service::start`]'s list of components, each given the same `Ctx`; an owner they register with `ctx.settings` and a topic
//! they set on `ctx.hub` appear in the API with no change to it.

use crate::api::Api;
use crate::config::{Config, Paths};
use crate::control::Stop;
use crate::ctx::Ctx;
use crate::fetcher;
use crate::homefeeds;
use crate::http::{self, Server};
use crate::hub::Hub;
use crate::preload;
use crate::receiver;
use crate::refreshguard;
use crate::screen;
use crate::settings::{SettingsStore, wake_on_change};
use crate::timeutil::now_ms;
use fetch::Fetch;
use serde_json::json;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How the service is started.
pub struct StartOptions {
    pub paths: Paths,
    pub config: Config,
    /// The HTTPS transport for weather and news. `None` builds the real one (`openssl s_client`) from the config; a test gives
    /// its own so nothing touches the network.
    pub fetcher: Option<Arc<dyn Fetch>>,
    /// What the AI-access module acts through. `None` builds the real ones (the TV's input devices, `luna-send`, the receiver);
    /// a test gives its own so nothing touches a TV.
    pub ai: Option<crate::ai::Deps>,
    /// Which addresses voice search, the weather and the plugins' Home Assistant lease may dial Home Assistant at. `None` is the home network
    /// and nothing else; a test that runs a fake Home Assistant on the loopback says so.
    pub voice_policy: Option<ha_link::AddressPolicy>,
    /// What the cameras act through. `None` builds the real ones (the home network only, RTSP over TCP, the staged feeder); a test names
    /// the loopback, its own feeder and dial, and quick timing.
    pub cameras: Option<crate::scrypted::Deps>,
    /// What On Home acts through. `None` builds the real ones (the pairing key of the Bluetooth proxy, the TV's LAN address, mDNS); a test
    /// names its own key and the loopback.
    pub onhome: Option<crate::onhome::Deps>,
}

/// A running helper.
pub struct Service {
    ctx: Ctx,
    ai: Option<Arc<crate::ai::Ai>>,
    server: Option<Server>,
    threads: Vec<(&'static str, JoinHandle<()>)>,
    addr: SocketAddr,
}

fn other(message: impl Into<String>) -> std::io::Error {
    std::io::Error::other(message.into())
}

impl Service {
    /// Starts every component and the API. Returns when the API is listening.
    pub fn start(options: StartOptions) -> std::io::Result<Self> {
        let StartOptions {
            paths,
            config,
            fetcher,
            ai: ai_deps,
            voice_policy,
            cameras: camera_deps,
            onhome: onhome_deps,
        } = options;
        let config = Arc::new(config);
        let stop = Stop::new();
        let hub = Arc::new(Hub::new(now_ms()));
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings
            .register_builtin()
            .map_err(|e| other(format!("settings: {e}")))?;
        crate::profiles::move_legacy_units(&paths.data_dir, |units| settings.adopt_weather_units(units));
        let ctx = Ctx {
            config: config.clone(),
            paths: paths.clone(),
            hub: hub.clone(),
            settings: settings.clone(),
            stop: stop.clone(),
        };
        // Ambient light exists from the start (its status and routes answer); its thread runs only while something asks for light.
        let ambient = crate::ambient::Ambient::new(&ctx);

        let (receiver_tx, receiver_rx) = mpsc::channel();
        let (feeds_tx, feeds_rx) = mpsc::channel();
        let (panel_tx, panel_rx) = refreshguard::channel();
        let guard = refreshguard::RefreshGuard::new(
            &ctx,
            Arc::new(refreshguard::LunaSource::new(&config.luna.send)),
        );

        // Stopping wakes everything that blocks: the runners on their channels, long polls on the hub.
        {
            let (r, f, p, hub, a) = (
                receiver_tx.clone(),
                feeds_tx.clone(),
                panel_tx.clone(),
                hub.clone(),
                ambient.clone(),
            );
            stop.on_stop(move || {
                let _ = r.send(receiver::Cmd::Stop);
                let _ = f.send(homefeeds::Cmd::Stop);
                let _ = p.send(refreshguard::Cmd::Stop);
                hub.close();
                a.wake();
            });
        }
        // A changed setting reaches the component that reads it at once.
        {
            let tx = receiver_tx.clone();
            wake_on_change(&settings, "base.receiver", &stop, move || {
                let _ = tx.send(receiver::Cmd::Wake);
            });
            for owner in ["base.weather", "base.news"] {
                let tx = feeds_tx.clone();
                wake_on_change(&settings, owner, &stop, move || {
                    let _ = tx.send(homefeeds::Cmd::Wake);
                });
            }
        }

        let mut threads: Vec<(&'static str, JoinHandle<()>)> = Vec::new();
        {
            let logging_ctx = ctx.clone();
            threads.push(("logging", thread::Builder::new().name("logging".into()).stack_size(128 * 1024).spawn(move || crate::logging::run(logging_ctx))?));
            let native_ctx = ctx.clone();
            threads.push(("logging-native", thread::Builder::new().name("logging-native".into()).stack_size(128 * 1024).spawn(move || crate::logging_native::run(native_ctx))?));
        }

        // The screen watcher tells both runners and ambient light when the panel goes on or off.
        let notify: Arc<dyn Fn() + Send + Sync> = {
            let (r, f, p, a) = (
                receiver_tx.clone(),
                feeds_tx.clone(),
                panel_tx.clone(),
                ambient.clone(),
            );
            Arc::new(move || {
                let _ = r.send(receiver::Cmd::Wake);
                let _ = f.send(homefeeds::Cmd::Wake);
                let _ = p.send(refreshguard::Cmd::Wake);
                a.wake();
            })
        };
        if let Some(handle) = screen::spawn(&ctx, notify)? {
            threads.push(("screen", handle));
        }
        threads.push((
            "receiver",
            receiver::spawn(&ctx, receiver_tx.clone(), receiver_rx)?,
        ));
        threads.push(("ambient", ambient.spawn()?));
        let (fetcher, fetch_backend) = match fetcher {
            Some(injected) => (injected, json!({"backend": "injected"})),
            None => {
                let chosen = fetcher::choose(&config.fetch);
                crate::log_info!(
                    "feeds: fetching through {} ({})",
                    chosen.backend,
                    chosen.detail
                );
                (
                    chosen.fetcher,
                    json!({"backend": chosen.backend, "why": chosen.detail}),
                )
            }
        };
        let wx = crate::wxview::WxView::new(Arc::clone(&fetcher), &paths.cache_dir());
        {
            let (hub, stop) = (ctx.hub.clone(), ctx.stop.clone());
            let (hub2, stop2) = (ctx.hub.clone(), ctx.stop.clone());
            let _ = (&stop2, &hub2);
            threads.push((
                "wx-prefetch",
                wx.prefetch(
                    {
                        let hub = hub.clone();
                        move || {
                            let w = hub.get(homefeeds::WEATHER_TOPIC)?;
                            Some((w.pointer("/doc/lat")?.as_f64()?, w.pointer("/doc/lon")?.as_f64()?))
                        }
                    },
                    {
                        let hub = hub.clone();
                        move || screen::is_on(&hub)
                    },
                    stop,
                )?,
            ));
        }
        let weather_ha = Arc::new(crate::ha_weather::HaWeather::real(
            &ctx,
            voice_policy.unwrap_or(ha_link::AddressPolicy::HOME),
        ));
        threads.push((
            "feeds",
            homefeeds::spawn(&ctx, feeds_rx, fetcher, weather_ha.clone())?,
        ));

        // AI access: loads its state (off and unpaired when there is none) and binds nothing; the LAN listener exists only while
        // the TV's owner has turned it on with the remote.
        let ai_deps = ai_deps.unwrap_or_else(|| real_ai_deps(&config, &settings, &guard));
        let nudge: Arc<dyn Fn() + Send + Sync> = {
            let tx = receiver_tx.clone();
            Arc::new(move || {
                let _ = tx.send(receiver::Cmd::Nudge);
            })
        };
        let ai = crate::ai::Ai::start(&ctx, ai_deps, nudge)?;

        // `base.livetv`, the tuner's old Settings switch, becomes the Live TV Tuner plugin's before the roster is first read.
        crate::tuner::migrate(&settings, &paths.plugins_dir());
        // Plugins: the manager reads the roster and keeps the broker's socket only while a plugin is switched on.
        let plugins = crate::plugins::Plugins::with_policy(
            &ctx,
            voice_policy.unwrap_or(ha_link::AddressPolicy::HOME),
        );
        threads.push(("plugins", plugins.spawn(&ctx)?));
        threads.push((
            "plugins-1p",
            crate::plugins::firstparty::spawn(&ctx, plugins.shared().clone(), feeds_tx.clone())?,
        ));
        threads.push(("preload", preload::spawn(&ctx)?));
        threads.push(("updateblock", crate::updateblock::spawn(&ctx)?));
        threads.push(("paneldim", crate::paneldim::spawn(&ctx)?));
        threads.push(("openapps", crate::openapps::spawn(&ctx)?));
        // Local profiles: the first open makes Nitin and Isabel from the working copy in theme/; the thread carries edits back to their profile.
        let profiles = crate::profiles::Profiles::open(&ctx);
        threads.push(("profiles", profiles.spawn(&ctx)?));
        threads.push((
            "refreshguard",
            refreshguard::spawn(ctx.stop.clone(), guard.clone(), panel_rx)?,
        ));
        // The Bluetooth proxy: nothing but one idle thread until the owner has linked it (`ble-link`).
        threads.push((
            "ble",
            crate::ble::spawn(&ctx, crate::ble::Deps::real(&config.luna.send))?,
        ));

        let ip: IpAddr = config
            .listen
            .host
            .parse()
            .map_err(|_| other("listen.host is not an address"))?;
        let voice = crate::voice::Voice::with_policy(
            &ctx,
            voice_policy.unwrap_or(ha_link::AddressPolicy::HOME),
        );
        // The screensaver's rules and slots: the owner's answers are read before anything is delivered.
        crate::saver::open(&ctx);
        // Cameras through Scrypted's LumaShow bridge: nothing connects until the TV is linked (`scrypted-link`), and nothing while it sleeps.
        let images = Arc::new(crate::images::Images::new(&config, &paths));
        let cameras = crate::scrypted::Scrypted::new(
            &ctx,
            camera_deps.unwrap_or_else(crate::scrypted::Deps::real),
            images.clone(),
            plugins.shared(),
        );
        threads.push(("scrypted", cameras.spawn(&ctx)?));
        // On Home: Home Assistant's shared entities, through the integration that dials the TV. Nothing listens until it is switched on and paired.
        let onhome = crate::onhome::OnHome::new(&ctx, onhome_deps.unwrap_or_else(|| crate::onhome::Deps::real(&ctx)));
        threads.push(("onhome", onhome.spawn(&ctx)?));
        // Live TV Tuner: the antenna tuner on the home network, a plugin of the helper's own. Nothing listens and the tuner is left alone
        // until the owner switches it on in Settings. Guide data: online listings laid over the TV's own guide, the tuner's `/xmltv.xml`
        // included, also off until switched on. Both read the TV's channel list through the same luna path.
        let tv: Arc<dyn lumatuner::Hardware> = Arc::new(lumatuner::webos::Tv::new(
            config.hdhr.webos(),
            Arc::new(lumatuner::webos::LunaSend::new(config.luna.send.clone())),
        ));
        // The Plex account: the TV's one sign-in to Plex, whose server Guide data's Plex source and the Photos plugin use. What Guide data
        // kept of a Plex server before it is the account's first.
        crate::plex::migrate(&paths, &settings);
        let (plex, plex_thread) = crate::plex::spawn(
            &ctx,
            Arc::new(crate::plex::CurlNet::new(&config.fetch)),
            crate::plex::Lending { settings: settings.clone(), shared: plugins.shared().clone() },
        )?;
        threads.push(("plex", plex_thread));
        let guide = Arc::new(crate::guidedata::GuideData::default());
        let tuner = Arc::new(
            lumatuner::Tuner::new(tv.clone(), lumatuner::real_clock()).with_guide(guide.clone()),
        );
        threads.push((
            "tuner",
            crate::tuner::spawn(&ctx, plugins.shared().clone(), tuner.clone())?,
        ));
        threads.push((
            "guide",
            crate::guidedata::spawn(&ctx, plugins.shared().clone(), tv, guide.clone(), plex.clone())?,
        ));

        let privacy = crate::privacy::new(&ctx);
        threads.push(("privacy", crate::privacy::spawn(&ctx, privacy.clone())?));
        // The display lab: a journal an earlier run left is put back before the API answers anything, and its thread is the dead-man.
        let displaylab = crate::displaylab::DisplayLab::start(&ctx);
        threads.push(("displaylab", displaylab.spawn(ctx.stop.clone())?));
        let api = Api {
            hub,
            settings,
            ai: ai.clone(),
            plugins,
            ambient,
            voice,
            images,
            cameras,
            onhome,
            privacy,
            capture: crate::capture::Capture::new(&config.capture),
            previews: crate::previews::Previews::new(&config, &paths, ctx.hub.clone()),
            displaylab,
            profiles,
            started_at: ctx.hub.boot(),
            data_dir: paths.data_dir.display().to_string(),
            fetch_backend,
            receiver_tx,
            feeds_tx,
            wx,
            weather_ha,
            guide,
            plex,
            tuner,
        };
        let server = Server::start(
            SocketAddr::new(ip, config.listen.port),
            Arc::new(api),
            http::Options {
                allowed_origins: config.allowed_origins.clone(),
                ..http::Options::default()
            },
        );
        let server = match server {
            Ok(server) => server,
            Err(e) => {
                // Nothing else is useful without the API: take the threads down again.
                let partial = Self {
                    ctx,
                    ai: Some(ai),
                    server: None,
                    threads,
                    addr: SocketAddr::new(ip, 0),
                };
                partial.stop();
                return Err(e);
            }
        };
        let addr = server.local_addr();
        crate::log_info!("listening on {addr} (api {})", crate::API_VERSION);
        Ok(Self {
            ctx,
            ai: Some(ai),
            server: Some(server),
            threads,
            addr,
        })
    }

    /// The address the API listens on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn hub(&self) -> &Arc<Hub> {
        &self.ctx.hub
    }

    pub fn settings(&self) -> &Arc<SettingsStore> {
        &self.ctx.settings
    }

    /// Stops everything: the stop is requested (long polls end, runners wake), the API stops accepting, and the threads get up
    /// to four seconds in all to finish a request they are in the middle of (a fetch can take ten; the process exit ends it).
    /// The whole stop stays under the eight seconds `lumatv-helper stop` allows before it uses SIGKILL.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.ctx.stop.request();
        // The LAN socket (if the owner turned AI access on) closes first.
        if let Some(ai) = self.ai.take() {
            ai.stop();
        }
        if let Some(server) = self.server.take() {
            server.stop(Duration::from_secs(2));
        }
        let deadline = Instant::now() + Duration::from_secs(4);
        for (name, handle) in self.threads.drain(..) {
            while !handle.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                crate::log_warn!("{name} did not stop in time and was left to the process exit");
            }
        }
    }
}

/// The real things the AI module acts through: the TV's input devices (to see the remote), `luna-send` (to read and drive the
/// TV), the input device that carries network keys (for the playback keys) and the Denon receiver. The TV's wakes go through `guard`.
fn real_ai_deps(
    config: &Config,
    settings: &Arc<SettingsStore>,
    guard: &Arc<refreshguard::RefreshGuard>,
) -> crate::ai::Deps {
    use crate::ai::keys::{EvdevInjector, EvdevKeys};
    use crate::ai::luna::{LunaTv, MediaKeyCodes, RealTv};
    use crate::ai::receiver_ctl::DenonControl;
    let ai = &config.ai;
    let mut luna = LunaTv::new(&config.luna.send);
    if let Some(id) = &config.brand_app_id {
        luna = luna.with_deny_prefixes(vec![id.clone()]);
    }
    let injector = Arc::new(EvdevInjector::with_dirs(
        &ai.sys_input_dir,
        &ai.input_dir,
        ai.inject_device.clone(),
    ));
    let codes = MediaKeyCodes {
        play: ai.media_keys.play,
        pause: ai.media_keys.pause,
        stop: ai.media_keys.stop,
        rewind: ai.media_keys.rewind,
        fast_forward: ai.media_keys.fast_forward,
    };
    crate::ai::Deps {
        keys: Arc::new(EvdevKeys::with_dirs(&ai.sys_input_dir, &ai.input_dir)),
        // The calls that wake a TV in standby wait out LG's panel refresh (`refreshguard`).
        tv: Arc::new(refreshguard::GuardedTv::new(
            Arc::new(RealTv::new(luna, injector, codes)),
            guard.clone(),
        )),
        receiver: Arc::new(DenonControl::new(settings.clone(), config.receiver.clone())),
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}
