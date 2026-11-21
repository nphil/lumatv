//! Where the Punktfunk core's log goes. The core and this engine log through `tracing`, which discards every event until a subscriber
//! is installed. The engine's standard output carries the protocol; its standard error is the shell's, which the app's feeder keeps in
//! `feeder.log` beside the other engines' lines, so each event becomes one line of standard error.
use std::{fmt::{self,Write as _},io::{self,Write},sync::{Mutex,atomic::{AtomicU64,Ordering}}};
use tracing::{Event,Metadata,Subscriber,field::{Field,Visit},level_filters::LevelFilter,span};

/// Warnings, errors and the info lines the core writes when something changes. Its debug lines come once per packet or frame.
const LEVEL:LevelFilter=LevelFilter::INFO;

/// Starts logging to standard error. Another subscriber already in place is left alone.
pub(crate) fn install() {
    let _=tracing::subscriber::set_global_default(Feeder::new(io::stderr()));
}

struct Feeder<W> {out:Mutex<W>,spans:AtomicU64}
impl<W:Write+Send+'static> Feeder<W> {
    fn new(out:W)->Self {Self {out:Mutex::new(out),spans:AtomicU64::new(1)}}
}
impl<W:Write+Send+'static> Subscriber for Feeder<W> {
    fn enabled(&self,meta:&Metadata<'_>)->bool {*meta.level()<=LEVEL}
    fn max_level_hint(&self)->Option<LevelFilter> {Some(LEVEL)}
    // Spans write no lines of their own: they are numbered so each has an id, and otherwise ignored.
    fn new_span(&self,_:&span::Attributes<'_>)->span::Id {span::Id::from_u64(self.spans.fetch_add(1,Ordering::Relaxed))}
    fn record(&self,_:&span::Id,_:&span::Record<'_>) {}
    fn record_follows_from(&self,_:&span::Id,_:&span::Id) {}
    fn enter(&self,_:&span::Id) {}
    fn exit(&self,_:&span::Id) {}
    fn event(&self,event:&Event<'_>) {
        let meta=event.metadata();
        let mut line=format!("punktfunk: {} {}:",meta.level(),meta.target());
        event.record(&mut Fields(&mut line));
        // One event is one line, whatever its message holds.
        let mut line=line.replace(['\n','\r']," ");
        line.push('\n');
        if let Ok(mut out)=self.out.lock() {let _=out.write_all(line.as_bytes());}
    }
}

/// The message as it is, then every other field as `name=value`.
struct Fields<'a>(&'a mut String);
impl Visit for Fields<'_> {
    fn record_debug(&mut self,field:&Field,value:&dyn fmt::Debug) {
        let _=if field.name()=="message" {write!(self.0," {value:?}")} else {write!(self.0," {}={value:?}",field.name())};
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone,Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl Write for Captured {
        fn write(&mut self,bytes:&[u8])->io::Result<usize> {self.0.lock().unwrap().extend_from_slice(bytes);Ok(bytes.len())}
        fn flush(&mut self)->io::Result<()> {Ok(())}
    }

    #[test]
    fn warnings_and_info_are_one_line_each_and_debug_is_left_out() {
        let captured=Captured::default();
        tracing::subscriber::with_default(Feeder::new(captured.clone()),|| {
            tracing::debug!("once per packet");
            tracing::info!(drops=3,"receive counters");
            tracing::warn!(target:"punktfunk_core::transport","receive buffer {} KiB is small",64);
            tracing::error!("two\nlines");
        });
        let text=String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert_eq!(text,concat!(
            "punktfunk: INFO lumagame_engine_punktfunk::logging::tests: receive counters drops=3\n",
            "punktfunk: WARN punktfunk_core::transport: receive buffer 64 KiB is small\n",
            "punktfunk: ERROR lumagame_engine_punktfunk::logging::tests: two lines\n",
        ));
    }
}
