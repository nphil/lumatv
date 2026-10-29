//! The screen contract (plan section 4.4): a screen's model is pure (`update(Input, now) -> Vec<Effect>`, `view()`, `due_at()`;
//! serde only, no I/O, tested on the host), and its surface draws the view with the kit. The host (`shell.rs`) owns the channel,
//! the helper and the clock, feeds the models and carries out their effects.

use serde_json::Value;

/// What the host tells a model.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// A topic of the UI channel: `audio`, `foreground`, `screen`, `cursor`, `refresh`, `transitions`, ...
    Topic { topic: String, data: Value },
    /// A luna answer to one of the model's calls, by the tag the model asked with.
    Luna { tag: u64, payload: Value },
    /// A topic of the helper's API (`receiver`, `cameras`, ...), and `up` (whether the helper answers) as a boolean.
    Helper { topic: String, data: Value },
    /// The answer to an [`Effect::Fetch`] (or an [`Effect::Theme`]): the status and the body as JSON.
    Fetched { tag: u64, status: u16, data: Value },
    /// The answer to an [`Effect::Fetch`] whose body is not JSON (a still, a thumbnail: JPEG or BMP, `kit::picture::decode`).
    Bytes { tag: u64, status: u16, bytes: Vec<u8> },
    /// A key of the remote by its name (`up`, `ok`, `back`, `0`..`9`, `volumeup`): pressed (`down`), repeated (`repeat`) or released.
    Key { name: String, down: bool, repeat: bool },
    /// The pointer: a sample while it is over the shell, or gone.
    Pointer { x: f32, y: f32, buttons: u32, visible: bool },
    /// Time passed: the host calls it at `due_at` and before any other input.
    Tick,
}

/// What a model asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// A luna call through the channel; the answers come back as `Input::Luna` with the tag.
    Luna {
        tag: u64,
        uri: String,
        payload: Value,
        subscribe: bool,
    },
    Cancel {
        tag: u64,
    },
    /// `POST` to the helper's API, the answer dropped.
    Post {
        path: String,
        body: Value,
    },
    /// `GET` from the helper's API; the answer comes back as [`Input::Fetched`] with the tag.
    Fetch {
        tag: u64,
        path: String,
    },
    /// A request of any method to the helper's API whose answer comes back as [`Input::Fetched`] with the tag: the status and the
    /// body of an error answer too (`Post` drops the answer; a wrong PIN is a 403 with the tries left in its body).
    Request { tag: u64, method: String, path: String, body: Option<Value> },
    /// A patch of the owner's theme (`theme/theme.json`), merged one level deep and written atomically by the host; the merged file
    /// comes back as [`Input::Fetched`] with the tag (status 200), or status 500 with the reason when it could not be written.
    Theme {
        tag: u64,
        patch: Value,
    },
    /// A line in the host's log (the journal on the set, stdout on the host).
    Log(String),
    /// The model's view changed: the surface is to be drawn again.
    Redraw,
}

/// A screen model.
pub trait Model {
    type View;

    fn update(&mut self, input: &Input, now_ms: u64) -> Vec<Effect>;
    fn view(&self) -> Self::View;
    /// When the model next wants a `Tick`, if it does (monotonic milliseconds).
    fn due_at(&self) -> Option<u64>;
}
