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
use crate::homefeeds;
use crate::http::{self, Server};
use crate::hub::Hub;
use crate::receiver;
use crate::screen;
use crate::settings::{SettingsStore, wake_on_change};
use crate::timeutil::now_ms;
use fetch::Fetch;
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
}

/// A running helper.
pub struct Service {
    ctx: Ctx,
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
        } = options;
        let config = Arc::new(config);
        let stop = Stop::new();
        let hub = Arc::new(Hub::new(now_ms()));
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings
            .register_builtin()
            .map_err(|e| other(format!("settings: {e}")))?;
        let ctx = Ctx {
            config: config.clone(),
            paths: paths.clone(),
            hub: hub.clone(),
            settings: settings.clone(),
            stop: stop.clone(),
        };

        let (receiver_tx, receiver_rx) = mpsc::channel();
        let (feeds_tx, feeds_rx) = mpsc::channel();

        // Stopping wakes everything that blocks: the runners on their channels, long polls on the hub.
        {
            let (r, f, hub) = (receiver_tx.clone(), feeds_tx.clone(), hub.clone());
            stop.on_stop(move || {
                let _ = r.send(receiver::Cmd::Stop);
                let _ = f.send(homefeeds::Cmd::Stop);
                hub.close();
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

        // The screen watcher tells both runners when the panel goes on or off.
        let notify: Arc<dyn Fn() + Send + Sync> = {
            let (r, f) = (receiver_tx.clone(), feeds_tx.clone());
            Arc::new(move || {
                let _ = r.send(receiver::Cmd::Wake);
                let _ = f.send(homefeeds::Cmd::Wake);
            })
        };
        if let Some(handle) = screen::spawn(&ctx, notify)? {
            threads.push(("screen", handle));
        }
        threads.push(("receiver", receiver::spawn(&ctx, receiver_rx)?));
        let fetcher = fetcher.unwrap_or_else(|| homefeeds::make_fetcher(&config.fetch));
        threads.push(("feeds", homefeeds::spawn(&ctx, feeds_rx, fetcher)?));

        let ip: IpAddr = config
            .listen
            .host
            .parse()
            .map_err(|_| other("listen.host is not an address"))?;
        let api = Api {
            hub,
            settings,
            started_at: ctx.hub.boot(),
            data_dir: paths.data_dir.display().to_string(),
            receiver_tx,
            feeds_tx,
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

    /// Stops everything: the stop is requested (long polls end, runners wake), the API stops accepting, and each thread gets
    /// up to five seconds in all to finish a request it is in the middle of (a fetch can take ten).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.ctx.stop.request();
        if let Some(server) = self.server.take() {
            server.stop(Duration::from_secs(3));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
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

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}
