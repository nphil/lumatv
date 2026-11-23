# LumaGame UI rework: build plan

Build order and file ownership for the screens chosen in [DECISIONS.md](DECISIONS.md). Layouts, copy and states come from the mocks in `mock/` (shot names below are the mock names); flows and cost estimates are in [OPTIONS.md](OPTIONS.md); the problems being fixed are in [AUDIT.md](AUDIT.md). Paths are under `rustui/app/src/game/` unless they start with `docs/`, `rustui/`, `game/`, `gfn/` or `tools/`.

Chosen: Home A Spotlight, Library B Panel, Game page C Glance, Steam A Accounts page plus the in-place sign-in row, Settings C Quick panel with the Settings A Rail behind "All settings", Launch C Strip, one quick menu on the Menu key. Release gate: 120 fps on the TV.

## 1. Rules for every slice

1. **Tokens only.** Colour, glass, radius, type and motion come from `rustui/tokens` (`focus.*`, `states.focus_wash`, `motion.*`). The mock CSS shows intent; its numbers are not copied into code. The durations OPTIONS names that the tokens lack (layer out 180 ms, art rest 250 ms, art dissolve 200 ms) are defined once in `screens/parts/motion.rs`. Plugins (Steam, GeForce NOW, PCs) supply data, never styling.
2. **Off limits:** `rustui/ui/src/kit/*` (LumaTV's kit), `rustui/render/src/glass.rs`, `rustui/render/src/shaders/glass.rs`. A change to the vendored `rustui/lumashow-ui` needs its matching `rustui/patches/ui/lumatv-src-*.patch`. LumaGame's shared pieces live in `screens/parts/`.
3. **Plugins stay optional.** With the Steam engine off or without `Feature::Community`, no community request, no sign-in row and no card for that data appear.
4. **Panel care.** A new element that can stay on screen states its dwell, brightness and movement in its commit.

   | Element | Rule |
   |---|---|
   | Hint bar | dims to 40 % after 5 s without a key |
   | In-game strip and figures pill | leave after 5 s |
   | Viewer caption and hints | leave after 3 s |
   | End-session confirm | keeps playing after 30 s without a key |
   | Starting screen (queue can last minutes) | logo and line dim to 40 % after 60 s; the ring keeps turning |
   | Everything else (Home art, logo, Play, top bar, panels, drawers) | the existing idle dim, then the saver (`screens/idle.rs`, unchanged); art and scrims are baked dim, no element brighter than the owner's SDR level |

   `AutoHide` (F1) is the one timer for the 5 s, 3 s and 30 s cases.
5. **Clean cutover.** A slice deletes the screens, model state, focus targets, shots and tests it replaces in the same series. No shims, aliases or dead code.
6. **Tests are consumer-visible:** keys and clock ticks in through `Model::step`; focus, commands and visible text out. Each runs well under a second on a fake clock. Behaviour covered once is not tested again.
7. **Worktrees.** One worktree per slice at `/data/home/lumaglass-wt-<slice>`, detached at `main` (no named branch; the repo commits straight to `main`); the shared checkout is not touched (it holds peers' uncommitted work). Builds and tests go through `run_on_worker` from the worktree: the first build there is cold, the worker keeps `target` afterwards (about 3.6 GB per worktree, removed when the slice lands). A sibling `target` per worktree is used because one shared `CARGO_TARGET_DIR` would serialise every build behind one lock. A slice rebases on `origin/main` before handing over; Main lands it on `main` and removes the worktree.
8. **Checks before handover.**
   - `cd rustui && cargo test --locked -p lumatv-rustui --lib`
   - `cd rustui && cargo clippy --locked -p ui -p lumatv-rustui --all-targets -- -D warnings`
   - The slice's shots: `GAME_SHOTS=<names>` writes `$CODY_SCRATCH/lumagame-native-shots/<name>.png`; the harness pairs each with the mock render in `$CODY_SCRATCH/lumagame-shots/<name>.png` into `<name>-compare.png`. Mock renders: `OUT=$CODY_SCRATCH/lumagame-shots node docs/lumagame/ux-rework/mock/render.mjs <names>` (Playwright from `/data/home/lucent-ha/node_modules`). On a worker, `CODY_SCRATCH` points inside the worktree and comes back through `results`.
   - The slice's budget walks (§6.3).
   - Match to the mock means same structure, copy and spacing within 8 px, colours from tokens; pixel equality is not the bar.
9. **Handover note** (the slice's final message, at most 15 lines): files changed, shots written, budget table, the long-dwell statement, one CHANGELOG paragraph, untested cases. Main merges, then edits `CHANGELOG.md` and `docs/lumagame/{ARCHITECTURE,HANDOFF}.md`; slices never edit these.
10. **Seams.** Foundation-owned files (§5) are read-only to slices once the foundation has landed. A slice that needs a change there sends Main one message naming the file, the change and the reason.
11. **TV.** Nothing here installs on the TV until §6.4. Only a commit on `main` at both remotes is installed; the lease and the owner's go come first (`docs/AGENTS.md`).

## 2. Target layout

```
game/
  mod.rs                 module list; F0 declares every file below, then nobody edits it
  input.rs               F1
  model.rs               core only: Model, Input, Command, Page, Preferences, step, key routing, engine events, persistence
  model/
    nav.rs               F0 creates the contract, F1 owns it: Layer stack, Region memory, Slice trait, Hints, AutoHide, Device
    text.rs              F0: text entry shared by search, host entry and diagnostics
    facts.rs             F3
    quick.rs             F2b
    refresh.rs           existing library refresh (read by F2b)
    tests.rs             F0: fixtures shared by model tests
    home.rs  library.rs  game.rs  settings.rs  accounts.rs  launch.rs      one slice each
    game/media.rs        Media
  screens/
    mod.rs  paint.rs     F2a: GameUi, Paint, enter/leave
    art.rs               F2c: stage, dip, pre-dimmed copy, upload throttle
    idle.rs  glyph.rs  symbols.rs     unchanged
    parts/{focus,topbar,hints,card,row,keyboard,drawer,dialog,strip,layer,rail,motion}.rs   F2a
    parts/stage.rs      F0 moves stage, logo, cover here with unchanged signatures; F2c owns the insides and adds over_art
    quick.rs             F2b
    home.rs  library.rs  game.rs  settings.rs  accounts.rs  launch.rs      one slice each
    lab.rs               Settings and Accounts (moves under Controllers)
    stats.rs             Launch
    game/media.rs        Media
  shots/
    mod.rs               F0: Shots harness; registers every module's screens() and walks()
    budget.rs            F2c
    fixtures.rs          F0 moves the base fixture, F3 adds the Hades community data
    foundation.rs        F2a: parts gallery
    quick.rs             F2b: quick menu shots
    home.rs  library.rs  game.rs  media.rs  settings.rs  accounts.rs  launch.rs
```

`Page` ends as `Home, Library, Game, Settings, Launch, Running`. `Layer` (replaces `Sheet`) is `Quick, Text, Game(..), Library(..), Settings(..), Accounts(..), Launch(..)`. `Focus` is `None, Nav(usize), Text(..), Home(..), Library(..), Game(..), Settings(..), Accounts(..), Launch(..)`, each inner enum owned by its slice. Variant lines in `Page`, `Layer`, `Focus`, `Command` and `Preferences` sit under one banner comment per slice so a slice edits only its own lines. Fallback if namespacing proves too heavy for F0: the shared enums stay flat, with the same banners.

## 3. Foundation (lands before the screen slices)

### 3.1 F0 · Split (alone, first) · size L

Precondition: the community work now uncommitted in `game/crates/lumagame-core/src/{community,engine,ipc,lib}.rs`, `game/engines/{steam,punktfunk}/` and the no-op arm in `model.rs` is committed on `main` by Main. F0 branches from that commit.

F0 moves code and fixes the contract. No behaviour or pixel changes.

| Today | Moves to |
|---|---|
| Focus `Hero`, `Home`; `rebuild_rows`, `HomeRow`/`HomeItem`, `home_row`; `pages::home` | `home` |
| Focus `Filter`, `Grid`, `SearchField`; filter, sort, genre, collection views; `Sheet::{Sort, Genre}`; `pages::library`; `Page::Search` | `library` |
| Focus `HubTop`, `HubAction`, `Tab`, `Way`, `Item`, `Remember`; `HubTab`, `model/hub.rs`; `Sheet::Chooser`; `play()`; `pages::{hub, way}` | `game` |
| Focus `Media`; `Page::Media`; media sheet; `Command::PlayMedia` | `game/media` |
| Focus `Rail`, `Setting`, `Theme`, `Lab`; `Section` renamed `Place`; settings pages incl. GameSir, Leadjoy, Lab, diagnostics, about; `Sheet::Theme`; `screens/{settings,lab}.rs` | `settings` |
| Focus `Integration`, `Target`, `EngineField`, `FirstRun`; `Page::Integration` renamed `Page::Accounts`; link flow, pairing, add and delete host; `Sheet::{Auth, Delete}`; `pages::first_run` | `accounts` |
| Focus `Overlay`, `Cancel`; `Page::{Launch, Running, Overlay}`; `open_overlay`, resume, end session; `Sheet::EndSession`; `screens/{launch,stats}.rs` | `launch` |
| Focus `Keyboard`; the text-entry state shared by search, add host and edit text; `Sheet::{AddHost, EditText}`; `pages::keyboard`. The entered text goes back to the slice named by the entry's kind | `text` plus `parts/keyboard.rs` |
| `pages::{nav, stage, logo, cover}`, `Paint` helpers | `parts/*` unchanged in look |

Also in F0:
- `shots.rs` becomes `shots/`: the harness stays in `mod.rs`, each page's current shots move into its slice file as `fn screens(base: &Model) -> Vec<(String, Model)>` (the `steam_screens` pattern), behavioural tests move with their slice, `carousel_motion_frames` splits between `home.rs` and `library.rs`.
- Model tests move into their slice files; helpers shared by several slices go to `model/tests.rs`.
- Cross-slice entry points exist with today's behaviour behind them (slices call these and implement their own):

  | Entry point | Owner | Callers |
  |---|---|---|
  | `Model::open_game(id, Option<facts::Section>)` | game | home, library, quick |
  | `Model::play(id)` | game | home, game |
  | `Model::open_settings(settings::Place)` | settings | quick, home (empty state) |
  | `Model::begin_auth(engine)` | accounts | game (sign-in row), settings |
  | `Model::refresh_libraries()` | `model/refresh.rs` | quick |
  | `Model::top_layer() -> Option<&Layer>` | F1 | every screen: the one reader of the open layer, so F1's move from one sheet to a stack touches no screen file |
  | `Model::last_failure(id) -> Option<&str>` | launch | game (Play becomes "Try again") |

- `model/nav.rs` carries the contract types (§3.5) with default methods, so F1 and F2a compile against one set of types. `facts::Section {Activity, Community, Media, About}` is created here for `open_game`.

Acceptance: `GAME_SHOTS=all` PNGs from the precondition commit and from F0 are identical (`cmp`); tests and clippy pass; every file in §2 exists and is declared.

### 3.2 F1 · Input and navigation · size M

Owns `input.rs`, `model/nav.rs`, the key-routing and `Layer` parts of `model.rs`, the pad mapping in `bin/lumagame.rs`, and in the separate `gfn/engine` workspace `crates/lumatv-gfn-pad/src/{menu,lib}.rs`.

- **Layer stack:** `View.layers: Vec<Layer>` replaces `sheet: Option<Sheet>`; `push(layer, focus)`, `pop()`; the focus beneath is restored on pop.
- **Regions:** each `Focus` enum implements `region()`; `View.memory` keeps the last focus per region; entering a region resumes it if still valid, else `Slice::default_focus`. Grids do not wrap. Held directions repeat at 300 ms, then every 100 ms, one retarget per frame.
- **Back ladder:** top layer closes; then focus returns to `Slice::anchor` (Play, or the last game); then the previous page; at Home's anchor the hint reads "Back again to leave" for 1.5 s and a second Back leaves (replaces today's immediate `Command::Exit`).
- **Menu key** (evdev 139, pad Y or Menu): F1 owns the rule `menu_allowed()`, true on every page and over game section layers, false over dialogs, the viewer, the keyboard, the Launch screens and a stream. The one `Key::Menu` arm that opens `Layer::Quick` is F2b's, so Menu keeps today's behaviour until the quick menu exists.
- **Keys:** `Key::{Prev, Next}` for pad LB and RB (section and Show-list steps; `Key::from_pad` ignores them today; `KEYS` in `bin/lumagame.rs` and `Key::from_menu` gain the two). In `lumatv-gfn-pad`: `MenuKey::{PageLeft, PageRight}` from `button::{LEFT_SHOULDER, RIGHT_SHOULDER}`, and Start held alone for 1 s emits the existing `PadEvent::Guide` beside the Back+Start exit chord (the chord wins). The streamer already turns Guide into its overlay request, and the menu sink into `Key::Guide`.
- **Stream:** Back, Guide or Start held opens the strip; while the strip is hidden, the first key only shows it (`AutoHide::swallow`).
- **Hints:** `Model.device` (Remote or Pad, from `Input::Key` vs `Input::PadKey`) and `Model.hints_dim` (true 5 s after the last key). The bar itself is F2a's.
- Tests (consumer-visible): the Back ladder in order including the 1.5 s Home arm; `menu_allowed` over each layer and page; region memory on leaving and re-entering; Prev/Next; held-Start emits one Guide, and the chord still wins (`gfn/engine`: `cargo test -p lumatv-gfn-pad`, on a worker); `AutoHide::swallow`.

### 3.3 F2 · Shared parts, art and quick menu · three agents

**F2a · parts (M).** Owns `screens/{mod,paint}.rs`, `screens/parts/*` except `stage.rs`, `shots/foundation.rs` (parts gallery).
- Enter/leave support in `GameUi::show/keep`: a node arrives (in 220 ms) and a removed node stays until its out (180 ms) ends; opacity, x, y and scale only. Reduced motion: 120 ms linear opacity or a jump.
- `parts` per §3.5. Focus treatment = `focus.*` scale and lift, `states.focus_wash`, `landing` underline. Layers are opaque sheets (`Backdrop::None`); glass over flat canvas is tint alone; glass over art is `Cached`, created with the page, never in a transition. `drawer` has one width for every side panel (the mock's `--rw-drawer`): quick menu, quick settings, accounts, in-stream menu.
- `parts/rail.rs`: the Settings A shell (title, five-place rail, pane frame with crumb and header chip, hints).
- Migrates the callers of the old `nav` to `parts::topbar` (Home, Library, Search and the profile avatar). The avatar's activation is F2b's.

**F2c · art and budget (M).** Owns `screens/art.rs`, `screens/parts/stage.rs`, `shots/budget.rs`, `Shots::frame()` in `shots/mod.rs`.
- Art change: focus rests 250 ms, then the dip (fade out 100 ms, swap, fade in 100 ms); one full-screen art layer at any time; nothing during a scroll.
- Pre-dimmed copy: the loader decodes a second, scrim-baked variant of the current art at decode time; `stage::over_art` dips to it when a layer opens and back when it closes. At most two stage-size images resident (current, next) plus the dimmed copy; memory only, nothing on disk; at most one texture upload per frame.
- Budget harness (§6.3): `Shots::frame()` returns the `FrameCost` that `engine.draw()` returns (today dropped); `Walk` runs a key script at `refresh_hz: 120.0` and reports peaks; the baseline run records today's numbers for every screen before any slice lands.

**F2b · quick menu (M, after F2a).** Owns `model/quick.rs`, `screens/quick.rs`, `shots/quick.rs` (`home-quick-menu`, `settings-c-quick`), the `Key::Menu` arm and the avatar's activation.
- Opaque drawer from the right, scrim only beside it, two views:
  - Menu: Refresh libraries ("Updated 2 min ago", or what is being checked; calls `refresh_libraries`); Accounts and PCs (the second line says what needs attention, for example "Desk PC is offline", with the warning mark; opens `open_settings(Place::Accounts)`); Settings (opens Quick settings); then a group headed by the highlighted game's title (`Slice::subject`): Game page, Pin to Home, Hide. Hint: Back, Close.
  - Quick settings (crumb back to Menu): Stream (stepper over the presets, "4K, 120 fps"), HDR (switch), Stats while playing (switch: off, or the last mode used; the three modes stay in All settings), Theme (stepper), Text size (stepper); an "All settings" button opens the Rail on its first place.
  - Rows reuse `parts/row.rs`. The five settings are exposed as `QuickSetting` so Launch's Picture panel reuses them.
- Wiring: `Key::Menu` opens `Layer::Quick` when `menu_allowed()`; in the same commit the two arms it replaces go (Library's Sort-on-Menu, Overlay's stats cycle). The top-bar avatar opens it too and carries the mark while `Model::attention()` (F3) is set.
- Pin to Home toggles the owner favourite (`user.favourites`); Home leads with pinned games. Hide inserts into `user.hidden`; Library shows a Hidden view with Unhide. A hidden game is out of every list.
- Tests: each row's action and result; steppers wrap and persist; Pin and Hide round trip; Menu again closes; Menu in Library and Overlay opens the quick menu.

### 3.4 F3 · Facts and community state · size M

Owns `model/facts.rs`, `shots/fixtures.rs`, the `Preferences.game_pages` field and its lines where preferences are read and written, `UserData.first_seen` in `game/crates/lumagame-core/src/state.rs`, and the one engine-event arm that today ignores `Event::Community` and `Event::CommunityThread`.

- `Model::facts(id) -> Facts` with one entry per `Section` plus `start`. `Fact<T>` is `Present(T)`, `Loading`, `Absent(Reason)` or `SignIn { expired }`. Reasons come from the engine's `Absent` (Empty, Unreachable, Unreadable, AgeGated, Other) and map to one set of words in `facts::why`, shared by Home and Game.
- Activity: play time, last session and where (history), achievements with rarity (from `Community.rarity`, tenths of a percent), players now, news. Community: reviews, discussions, guides. Media: store screenshots and trailers; community screenshots, artwork, videos. About: description, developer, release, controller support, genres. Friends who play are not offered (no source).
- `start: Start {Ready, Cannot(Why)}`: an asleep PC counts as Ready and wakes on Play; only a game that cannot start now carries a mark. Words: Ready, Asleep, Offline, Sign in, Pair.
- `Model::attention() -> Option<Attention>`: an enabled engine needs sign-in or pairing, or its last refresh failed. `Attention` carries the one line in words ("Desk PC is offline"). The top bar and the quick menu read it.
- Fetch: for a game with a Steam id, one `Request::Community { mature: false }` per game when focus rests on it or its page opens, through the existing `Command::Send`; one in flight per game; answers kept for the session and refreshed after 10 minutes. Thread requests are fetched when a topic opens. Everything is gated by `prefs.game_pages` (default on) and `Feature::Community`.
- `added`: a first-seen time per game for "Recently added" (`UserData.first_seen`, defaulting empty so the schema number stays; set when a game first appears after the first refresh completes; games present at first import carry none).
- Tests: Absent reasons to words; fetch only for Steam games, once, and not when the switch is off or the engine lacks the feature; an event updates facts; first-seen rules; the `attention` truth table.

### 3.5 Contract (the shape slices code against)

Names are final when the owner merges; the shape is fixed now.

```rust
// model/nav.rs (F0 types, F1 logic)
pub trait Slice {                                          // one impl per slice, dispatched on Page and Layer
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>>;           // the focus grid, one Vec per row
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>);
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool;
    fn default_focus(m: &Model, region: Region) -> Option<Focus>;          // first focus of a region with no memory
    fn anchor(m: &Model) -> Option<Focus> { None }         // where Back goes before it leaves the screen
    fn subject(m: &Model) -> Option<GameId> { None }       // the highlighted game, for the quick menu
    fn hints(m: &Model) -> Hints { Hints::default() }      // at most three: key plus one word
}
pub struct AutoHide { /* ms, until */ }                    // show(now), visible(now), swallow(now) -> bool
pub enum Device { Remote, Pad }

// screens/parts (F2a), all take &mut Paint and read tokens
focus::treat(p, id, frame, Kind::{Tile, Card, Row, Button}, focused)   // lift, wash, landing; reduced motion: wash and landing only
topbar::bar(p, here, attention)                                        // Home, Library, Search pill and the profile avatar
hints::bar(p, &Hints, Side::{Right, Left}, dim)                        // glyphs follow Model.device
card::glance(p, &Card, frame, focused)                                 // label, headline, detail, "From Steam" foot, progress, thumbs
row::{link, value, switch, stepper}(p, id, frame, ..)
drawer::open(p, side, width, title) -> Frame                           // opaque sheet; scrim only beside it
dialog::modal(p, title, body, actions, safe)                           // opaque backdrop; focus starts on `safe`
strip::bar(p, items, focused) -> Frame
layer::section(p, sections, current) -> Frame                          // section shell: names, content slot, 180 ms fade
stage::{art, logo, cover, over_art}(p, ..)                              // F2c; over_art dips to the pre-dimmed art
rail::shell(p, &RailSpec) -> Frame                                      // Settings A shell
keyboard::plan(p, &TextEntry)
motion::{LAYER_IN_MS, LAYER_OUT_MS, ART_REST_MS, ART_DIP_MS, ..}       // from tokens.motion where they exist

// model/facts.rs (F3)
pub enum Section { Activity, Community, Media, About }
pub enum Fact<T> { Present(T), Loading, Absent(Reason), SignIn { expired: bool } }
impl Model { pub fn facts(&self, id: GameId) -> Facts; pub fn open_thread(&mut self, id: GameId, topic: TopicId, out: &mut Vec<Command>); }
pub struct Attention { pub words: String }                             // Model::attention() -> Option<Attention>

// shots/<slice>.rs (each slice)
pub(super) fn screens(base: &Model) -> Vec<(String, Model)>;
pub(super) fn walks() -> Vec<Walk>;                                    // Walk from shots/budget.rs
```

## 4. Screen slices

Each slice starts when F1, F2a, F2c and F3 are on `main`. Every slice's brief opens with: read `DECISIONS.md`, its `OPTIONS.md` section, its mock source in `mock/`, §1 and its own section here. Shot names match the mock PNG names; state shots add a suffix (`-text200`, `-light`, `-empty`). Dark and light at 100 % text, plus 200 % text on the densest screen, where a mock exists for comparison and where one does not (light mocks exist only for Home A and Game B).

### 4.1 Home (A · Spotlight) · size M

- **Owns:** `model/home.rs`, `screens/home.rs`, `shots/home.rs`.
- **Builds:**
  - Top bar from `parts::topbar` (Home, Library, Search pill; profile avatar opens the quick menu and carries the attention mark).
  - One tile row, at most 40 games: pinned, then recent, then most played, one entry per game. The row slides under a fixed focus slot; the focused tile is larger with the underline.
  - Stage: the focused game's art (the dip, after focus rests 250 ms, never in a scroll), logo, one line ("Played 2 h ago · 31 h · 18 of 50 achievements", an unknown part left out), Play with the where-to-play glyph. A mark on a tile only when the game cannot start now.
  - Down swaps the row for the card strip over 220 ms: Achievements (18 of 50 and three icons), Last played, News, Reviews, each footed "From Steam". A card appears only when its fact is present, so a game with no Steam record shows two. Up returns. OK on a card opens that game page at that section; OK on a tile or Play plays (`Model::play`).
  - Empty state: one line and one button, "Add accounts and PCs" (`open_settings(Place::Accounts)`).
  - Hints: More (Down), Menu; on the card strip Games (Up), Menu.
- **Removes:** the Hero buttons (including Refresh), the shelves (Most played, Favourites, Collections, Genres, Native on this TV, Library), `HomeRow`/`HomeItem`, `home_row`, `HOME_ROW_MAX`, `pages::home`, and `Preferences.hero` (the "Featured game" size; Settings has dropped its row by merge order). Collections, genres and "Native on this TV" are reached from Library's Show and Refine lists.
- **Data:** `Model::facts(id)` including `start`; `state.profile().user.favourites` for pins; history for recent and most played.
- **Shots:** `home-a-spotlight`, `home-a-spotlight-down`, `home-a-spotlight-light`, plus `-text200`, a no-Steam-record game, and the empty state.
- **Walks:** `home-right-walk` (tile steps with art rests), `home-down-up`.
- **Tests:** Right moves one tile and the backdrop target changes only after the rest; Down shows only present cards and Up returns to the same tile; OK on Play issues the play command; a game with Steam facts absent shows the Last played card only; the empty state's button opens Settings on Accounts; Back at the anchor arms leave.

### 4.2 Library (B · Panel) · size M

- **Owns:** `model/library.rs`, `screens/library.rs`, `shots/library.rs`.
- **Builds:**
  - Left panel: Search row first (opens the keyboard layer with live results), Show (All games, Favourites, Recently added, Ready now, one row per collection, Hidden when any, each with a count), Refine (Plays on, Genre, Sort; each opens a list layer from `parts::drawer`).
  - Grid on the right, per the cover preference, with a caption under the focused cover (name, "58 h played"); Left from column 0 enters the panel, Right returns to the last grid focus; LB and RB step the Show list.
  - Top-bar Search opens this page with the keyboard open; `Page::Search` is deleted.
  - States: empty library (one line and the Accounts button), no results.
  - Hints: Open, Filters.
- **Removes:** filter chips, the refresh chip, `Page::Search`, `Sheet::{Sort, Genre}` as bare sheets, `pages::library` (F2b has already replaced the Menu-key Sort arm).
- **Data:** counts computed once per catalog rebuild; `facts.start` for Ready now; `facts.added` for Recently added (14 days); `user.hidden`, collections, favourites.
- **Shots:** `library-b-panel`, with `-light`, `-text200`, an open Refine list, the keyboard, `library-empty`, `library-no-results`.
- **Walks:** `library-grid-scroll`, `library-panel-in-out`.
- **Tests:** Show row changes the grid and count; Refine filters compose; Left and Right memory; search narrows live and Back leaves the keyboard before the page; empty and no-results states; Unhide returns a game.

### 4.3 Game page (C · Glance) · size L

- **Owns:** `model/game.rs` (including today's `model/hub.rs`), `screens/game.rs`, `shots/game.rs`.
- **Builds:**
  - Page: full art, logo, line, Play and the where-button ("GeForce NOW ⌄"), four cards in order Activity, Community, Media (image card), About. Hints: More, Menu.
  - Where-button opens the chooser layer: ways to play with the five words, Remember, "Sign in" and "Pair" rows that begin their flows. A failed start turns Play into "Try again" with the reason on one line (`last_failure`).
  - A card opens its section as a layer over the pre-dimmed art (`stage::over_art`, no scrim layer). The layer shows the four section names; Left and Right (LB and RB) switch, with a 180 ms content fade; Back closes and focus returns to the card.
    - Activity: play time, last session, achievements with rarity, players now, news.
    - Community: reviews, discussions, guides.
    - About: description, developer, release, controller support.
    - Media: body supplied by the Media slice.
  - Reader layer: 28 px text at 100 % (scales with Text size), about 60 characters a line, paged with a counter; opens for a news item, a review, a discussion thread (`open_thread`), a guide summary (guides carry a summary only) and the About text. Back closes the reader, not the section.
  - In-place sign-in row at the top of Activity when Steam is enabled and signed out ("Sign in to Steam to see achievements") or its sign-in has expired ("Steam sign-in has expired" with "Sign in again"); OK calls `begin_auth`. Absent without the Steam engine.
  - A fact that is Absent says why in place, in one line, never an empty box.
- **Removes:** `HubTab` and tabs (`Focus::Tab`), `hub_tab`, Ways and Item rows, `HubTop`/`HubAction`, `pages::{hub, way}`, the old chooser sheet, hub news and details rendering and anything only they used. Fields in `lumagame-core` IPC that nothing reads afterwards are listed in the handover for Main's cleanup (§7).
- **Data:** `Model::facts`; `open_thread`; ways from `Game.sources` with readiness; `prefs.game_pages`.
- **Shots:** `game-c-glance`, `game-c-glance-text200`, `game-c-glance-community`, `game-b-split-reader`, `steam-b-in-place`, `steam-b-in-place-stale`; states: no Steam record, offline, absent reasons, Try again, chooser; `-light`.
- **Walks:** `game-section-open-close`, `game-section-switch`, `game-reader-page`.
- **Tests:** OK on a card opens that section and Back returns to that card, then Back leaves the page; section switch keeps each section's focus; a news item opens the reader and Down pages it; signed-out Steam shows the row and OK begins auth, Steam off shows no row; each Absent reason shows its words; the chooser's Remember sets the remembered source.

### 4.4 Media (hub media and viewer) · size M

- **Owns:** `model/game/media.rs`, `screens/game/media.rs`, `shots/media.rs`. Works against the section shell (`parts::layer::section`) and the `media::` entry points F0 creates (`rows`, `activate`, `adjust`, `section`, `viewer`, `hints`), so it does not wait for Game; the integrated shots are retaken after Game merges.
- **Builds:**
  - Media section body: store trailers and screenshots, with filters for community screenshots, artwork and videos; a thumbnail grid.
  - Viewer layer: full screen, Left and Right step through items and stop at the ends, caption and hints leave after 3 s (`AutoHide`) and the first key brings them back, Back closes. One image decode at a time, neighbour prefetched while resting, at most one upload per frame.
  - Trailers play through the existing `Command::PlayMedia`. Community videos link to YouTube: the default is a hand-off to the TV's YouTube app (`FromApp::Launch` with `youtube.leanback.v4` and the video id) through one new `Command::OpenApp { app_id, parameters }`; Back returns to LumaGame. See open question 1.
- **Removes:** `Page::Media`, `Focus::Media`, the media sheet and its list, the old media code in `pages`.
- **Data:** `facts.media`; `Command::FetchArt` for thumbnails and full images.
- **Shots:** `game-viewer`, `game-viewer-chrome-hidden`, the Media section, loading, none, error.
- **Walks:** `media-viewer-step`.
- **Tests:** filter switch; OK opens the viewer at the index; stepping stops at both ends; caption hides after 3 s and the first key only reveals it; Back order (viewer, then section); a video item emits `Command::OpenApp` with its id.
- **Seam:** `Command::OpenApp` and its handler in `bin/lumagame.rs` (the shell side forwards `FromApp::Launch`, which exists) are the only foundation edit; Media sends the request to Main.

### 4.5 Settings and Accounts (Rail layout, Accounts and PCs) · size L

One agent: the two share the Rail shell, the page host and the dialogs. Order: shell and places first, then the Accounts pane and dialogs. If Main wants two agents, Accounts starts after the shell's first merge.

- **Owns:** `model/{settings,accounts}.rs`, `screens/{settings,accounts,lab}.rs`, `shots/{settings,accounts}.rs`.
- **Builds (Settings):**
  - `Page::Settings` on `parts::rail::shell` with five places: Accounts and PCs, Picture and stream, Controllers, Appearance, System. Diagnostics and About live under System; GameSir, Leadjoy and the controller lab are panes of Controllers (the lab's pad mode follows the pane, not a `Page`); one theme picker under Appearance, without the "Featured game" row.
  - Rows from `parts::row`; every place scrolls inside the pane; Left returns to the rail.
  - Hints as in the mock: the row's own OK word, and Back.
- **Builds (Accounts):**
  - Accounts and PCs place: services and PCs with state chips in the five words (an account reads Linked or Sign in); each opens its own pane. Steam pane rows as in the mock: state chip, Account, Games matched ("118 of 142"), Updated, News and reviews ("On game pages", the `prefs.game_pages` switch; the engine schema has no such field), Sign out. PC panes: pair, add host, delete, "No PCs found". GeForce NOW: sign in and out. This TV: app choice.
  - Sign-in dialog (`parts::dialog`): white QR plate, code, ring countdown, opaque backdrop, Back cancels; failure shows the reason and "Try again". The in-place row on game pages calls `begin_auth`, so the dialog renders over any page.
  - The first-run screen becomes this place's empty state.
- **Removes:** the old `Section` list and its pages, the Integrations list, `Page::Accounts` (F0's rename of `Page::Integration`), `Page::{Lab, GameSir, Leadjoy}`, `Sheet::Theme` and the duplicate theme controls, the "Featured game" row, the bare auth, pair and add-host sheets, `pages::first_run`.
- **Data:** integration and link-flow state; matched counts (games with a source or report from the engine against all games); `refresh.rs` timestamps; `prefs`.
- **Shots:** `settings-a-rail`, `steam-a-accounts-page`, `steam-a-accounts-page-qr`, `steam-b-in-place-qr` (dialog over a game page); states: Steam stale, sign-in failure, "No PCs found", pairing PIN, add host, empty first run; `-light`, `-text200`.
- **Walks:** `settings-rail-walk`, `accounts-dialog-open-close`.
- **Tests:** place switching and memory; stepper and switch rows persist; sign-in begin, QR, linked; failure shows reason and Try again; Sign out; matched counts; the switch writes `game_pages`; Back from a pane returns to the rail.

### 4.6 Launch and in-game strip (C · Strip) · size M

- **Owns:** `model/launch.rs`, `screens/{launch,stats}.rs`, `shots/launch.rs`.
- **Builds:**
  - Starting: logo, ring, one line naming the step in words ("In queue, 14 ahead", "GeForce NOW, about 2 min"); Cancel as a Back hint. Only the steps a way has are shown. Dims after 60 s.
  - In a game: bottom pill (Resume, Picture, Controller, Achievements, End) and a top-right figures pill (4K, 120 fps, 18 ms) that both leave after 5 s; the first key brings them back and does nothing else (`AutoHide::swallow`). Picture opens the quick settings Stream, HDR and Stats rows (`QuickSetting`) on a drawer; Controller opens controller settings (`Command::OpenControllerSettings`); Achievements lists them from `facts`. Layers over a stream are opaque sheets, with a scrim only beside the drawer.
  - End: confirm dialog on an opaque backdrop, focus on "Keep playing", closes itself after 30 s. Steam overlay on X (`Key::Alt`), only for copies that have it.
  - `last_failure(id)`.
- **Removes:** `Page::Overlay` and its list menu, the launch chips, `Sheet::EndSession` as a sheet, the old `open_overlay` list.
- **Data:** session state from the engine events; `QuickSetting`; `facts.activity` achievements.
- **Shots:** `launch-c-strip`, `launch-c-strip-menu`, starting (queue, wake, signing in), failure, end-session confirm (an AUDIT gap), figures pill; `-light`, `-text200`.
- **Walks:** `launch-start`, `strip-in-out`.
- **Tests:** each launch step's line; Back cancels a start; the strip shows, hides after 5 s on the clock, and the first key only reveals it; Picture rows change prefs; End asks first and Keep playing is the default; the 30 s timeout resumes; the Steam overlay key acts only when the copy has it.
- **Untested until a real session:** the strip over a live stream.

### 4.7 G1 · Gate surfaces · size S to M

- **Owns:** `docs/research/perf/gate.py` only.
- **G1a (starts now, needs the lease and the owner's go):** `game-*` entries for `org.nphil.lumagame` on today's UI (Home walk, Library scroll, game page open, Settings walk), recording the before numbers. Each entry follows the `store-scroll` precedent: setup closes and launches the app (`luna-send ... closeByAppId`, `launch`), teardown relaunches `com.webos.app.home`, `app_retries: 0`, `thresholds: {'burst_end_ms': None}`, a `prep` that walks to the screen's anchor with D-pad moves (never two Backs at Home, which now leaves the app).
- **G1b (after the slices merge):** rewrite the key paths for the new UI and add: Home tiles and Down/Up, Library grid and panel, game section open, close and switch, viewer, quick drawer open and close, Rail walk, Accounts dialog, launch (starting) and strip (the strip over a live stream needs a real session and stays untested until the owner runs one).

## 5. Conflict map

| File | Why it conflicts today | After F0 | Editors afterwards |
|---|---|---|---|
| `model.rs` | 4296 lines: every screen's state, focus, activation, back, 1400 lines of tests | Core only | F1 (key routing, layers), F2b (the `Key::Menu` arm), F3 (`game_pages`, one engine-event arm); a slice edits only its own banner lines in `Page`, `Command` and `Preferences` |
| `model/<slice>.rs` | did not exist | each slice's state, `Focus`, `Layer`, `Slice` impl, tests | its slice only |
| `model/nav.rs`, `input.rs`, `bin/lumagame.rs` pad mapping, `lumatv-gfn-pad` | one input path | contract types | F1 only; Media's `Command::OpenApp` through Main |
| `game/mod.rs` | module list | declares every file in §2 | nobody |
| `screens/mod.rs` | `GameUi`, page dispatch, `keep` animations, hit tests | dispatch is a `match` on `Page`/`Layer` calling `<slice>::plan` | F2a only |
| `screens/paint.rs`, `screens/parts/*` except `stage.rs` | `Paint` shared by every screen | same, plus `parts` | F2a; slices compose from `parts` and add nothing |
| `screens/art.rs`, `screens/parts/stage.rs` | stage and loader | same | F2c only |
| `shots.rs` | 1450 lines: harness, every screen, tests | `shots/` | `mod.rs`: F0 then F2c (`frame()` only); each other file its owner |
| `rustui/tokens/src/lib.rs` | shared with LumaTV | unchanged | nobody; a gap goes to Main, and a change must keep the shell's shots identical |
| `lumagame-core/src/state.rs` | saved profile | unchanged | F3 only (`UserData.first_seen`) |
| `CHANGELOG.md`, `docs/lumagame/*` | shared docs | | Main |
| `docs/research/perf/gate.py` | gate surfaces | | G1 only |

A slice that finds a needed change in a foundation file stops and asks (§1.10). Expected traffic is low: `Command::OpenApp` is the only one known.

## 6. 120 Hz plan

### 6.1 Cheaper variants adopted

| Transition | Variant | What it avoids |
|---|---|---|
| Home art change | dip: fade out 100 ms, swap, fade in 100 ms, after a 250 ms rest | two full-screen art layers during a dissolve |
| Game section opens | art dips to its pre-dimmed copy, no scrim layer | a full-screen scrim blend (about 3 Mpx) |
| Quick drawer | opaque sheet, scrim only beside it (OPTIONS: 0.9 Mpx) | a second full-screen layer over Home |
| Dialogs, strip, drawers over a stream | opaque sheets, no glass, no backdrop capture | a cold capture of 12 to 15 ms (`frames-120-result.md`) |
| Library panel and bars | tint-only glass over canvas; `Cached` glass over art, created with the page | capture inside a transition |
| Page to page | cut; only content that arrives inside a page fades (180 ms, its own layers) | full-screen fades that redraw every layer (the kit sets its modal fade to 0 from 100 Hz) |
| Art load | one texture upload per frame, neighbour prefetched while resting | upload stalls in a scroll |
| All motion | x, y, scale and opacity only; nothing reflows during a move | layout work per frame |

### 6.2 Layer budgets

Mock-counted from OPTIONS; the engine's layers differ, so F2c's baseline calibrates them. Design ceiling on the host: 44 drawn layers per frame. The TV cost model (`docs/research/frames-120-result.md`: GPU about 1.5 ms plus 0.085 ms per drawn layer, pixels add almost nothing) puts the cliff near 80 layers at 8.33 ms. The ceiling moves only with a TV measurement. The origin of OPTIONS's "6.6 Mpx" figure is not documented, so pixels are reported, not gated.

| Screen and state | Mock layers | Full-screen layers | Rule |
|---|---|---|---|
| Home A at rest / Down | 16 / at most 20 | 1 | one art layer; dip changes only its alpha |
| Library B | 30 | 0 | 2 cached glass sheets; one upload per frame while scrolling |
| Game C at rest / section open | 10 / 22 | 1 | pre-dimmed copy, no scrim |
| Settings Rail with pane | 18 | 0 | two tint sheets over a flat canvas |
| Quick drawer over Home A | 22 | 1 | scrim beside the drawer only |
| Dialog over a page | page plus 4 | 1 opaque | no capture |
| Launch starting / in a game | 3 / 4 | 0 | ring, one line; strip is a pill |

### 6.3 Host self-check

F2c's `shots/budget.rs`:
- `Shots::frame()` returns `FrameCost` (`layers`, `region_px`, `damage_px`, `rects`, `draw_calls`, `uploaded_bytes`, `backdrops`, `moving`).
- `Walk { name, base: Model, script }` steps keys and waits at `refresh_hz: 120.0` and reports peaks. A walk fails when a frame draws more than 44 layers, uploads more than one stage-size texture, or captures a backdrop outside the first frame of a page. It also fails when two full-screen layers draw in one frame, which the dip and the pre-dimmed copy rule out.
- Runs are explicit, like `GAME_SHOTS`: `GAME_BUDGET=all` or a comma list of walk names; `GAME_MOTION=right|down|grid` and `GAME_MOTION_REDUCED` keep writing frame sequences to `$CODY_SCRATCH/lumagame-motion`. Software GL makes `cpu_us` and `gpu_us` meaningless on the host; only the deterministic fields are gated.
- Each slice registers its walks in its own `shots/<slice>.rs` and puts the table of peaks in its handover. Walks per slice are named in §4.
- A host pass does not show 120 Hz on the TV. It stops the known causes (layer count, full-screen overdraw, uploads, captures) before the TV does.

### 6.4 TV gate (G2)

The UI is its own LG card window (`org.nphil.lumagame`) with its own `Engine` at `refresh_hz: 120.0`; it is drawn under whichever compositor runs, and no LumaGame surface exists in `gate.py` today (HANDOFF "Next work" 5). G1 supplies them. After the last slice merges:

1. Main builds from the pushed `main` commit: `rustui/tools/build-game.sh`, then `game/build-ipk.sh` (the build id is a hash of the packed files; UI binary `bin64/lumagame-ui`). The commit is on `main` at `origin` and `forge`. Record the commit and the binary hashes.
2. Take the TV: `/data/home/Marquee/tools/tv-lock.sh acquire --why "LumaGame 120 Hz gate"`. If it reports IN USE, the owner says go first; read-only while the owner is watching. Receiver muted.
3. Install: `tools/lumatv game install`, then `tools/lumatv game status`; the build id and the installed `lumagame-ui` hash match step 1.
4. `compositor/deploy/gate.sh refresh 120` (panel at 119.9 Hz). Regression check of the shell: `python3 compositor/tools/tv_gate.py --only frames --frames-quick --refresh 120`.
5. `docs/research/perf/gate.py --surfaces <the game-* surfaces> --input-key 241 --cycles 3 --refresh 120` (pop-ups are turned off and back on by the gate).
6. Pass, from the gate's own thresholds: `burst_over20` 0; `dropped` 0 (under LG's compositor the gate holds it to 17 % of the animated frames instead, as in `gate.py`); `p90_render_ms` at most 3.0; `p90_window_ms` at most 4.0; `first_frame_ms` at most 50 (G1b sets 15 for a layer opening on a standing page). G1a's baseline is the comparison; no threshold is loosened.
7. `tools/lumatv video-check` after the install and the runs.
8. Release the lease. Record the receipt path in the handoff and list what was not tested: the strip over a live stream, the YouTube hand-off, a pad session for LB, RB and held Start, and any surface the owner did not allow.

A host `GAME_SHOTS=all` pass and `GAME_BUDGET=all` precede the install. No claim of 120 Hz is made before step 6 passes on the TV.

## 7. Order, sizes and commands

```
now          Main: commit the community work       G1a: baseline on the TV (lease, owner's go)
then         F0 split                                              L
then         F1 input and navigation          M     F2a parts   M     F2c art and budget   M     F3 facts   M
then         F2b quick menu (after F2a)       M
then         Home M · Library M · Game L · Media M · Settings and Accounts L · Launch M
             (each needs F1, F2a, F3 on main and F2c for the budget walks; Home, Library and Launch merge after F2b)
integration  merge order: F2b, Settings and Accounts, Launch, Library, Home, Game, Media
after        G1b, cleanup, host passes, IPK, G2 (TV)
```

- **Merge order reasons:** F2b first, because Home and Library delete their own Refresh controls and Launch reuses `QuickSetting`; Settings and Accounts early, because Home's empty state and the quick menu route into it; Game before Media for the integrated viewer shots.
- **Each merge:** Main rebases the slice on `main`, re-runs the two commands below on the merged result, and pushes `main` to `origin` and `forge`.
  - `cd rustui && cargo test --locked -p lumatv-rustui --lib`
  - `cd rustui && cargo clippy --locked -p ui -p lumatv-rustui --all-targets -- -D warnings`
- **After the last merge:** `GAME_SHOTS=all` for the full set of screens compared with the mocks, `GAME_BUDGET=all`, then G1b and G2. Main removes IPC fields nothing reads any more (checked with `lsp` references; core and engines change together), updates `CHANGELOG.md`, `ARCHITECTURE.md` and `HANDOFF.md` from the handover paragraphs, and removes the worktrees.
- Every merge passes tests and clippy. Nothing is installed on the TV before G2, so a transitional UI between merges is acceptable; each slice keeps its own screens working.

## 8. Decided where the options were silent

- Pin to Home is the owner favourite; Home leads with pinned games, and Library's Favourites shows the same set. Hide uses `user.hidden`, with an Unhide path in Library.
- Search is the Library page with the keyboard open.
- Collections are Show rows; genres are a Refine row; "Native on this TV" is a "Plays on" choice.
- Friends who play are left out (the Steam engine has no friends data).
- "News and reviews" is a UI-side switch (`prefs.game_pages`, default on); the engine schema has no such field.
- An asleep PC counts as Ready. Only a game that cannot start now carries a mark.
- Page changes are cuts at 120 Hz; the dip is the only art transition.
- The quick menu stays closed over dialogs, the viewer, the keyboard, Launch and a stream.
- Guides open as a summary reader, because the engine returns summaries only.
- The quick menu has two views: Menu, and Quick settings behind its Settings row. Stats while playing is a switch there; Off, Compact and Full stay in All settings.
- The "Featured game" size setting goes with Home A; its Settings row goes with the Settings slice.

## 9. Owner answers (2026-11-23)

1. **Videos play inside LumaGame**, embedded, for both store trailers and Community Hub videos; Back returns to the page. The playback route (native HLS for trailers, YouTube's official embedded player for community videos) is being researched; the Media slice builds on its result. Extracting YouTube streams directly is out: it breaks YouTube's terms.
2. **Mature game pages** is a switch in Settings (Accounts and PCs › Steam pane), off by default. On, community requests send `mature: true`; off, age-gated parts show "Age-restricted on Steam". Owned by the Settings and Accounts slice; F3 reads it when building the request.
3. **All moving media plays on its own display layer** (hardware video plane or a separate compositor surface), never decoded into UI textures: trailers, YouTube videos, animated GIF/WebP/WebM capsules and any looping clip. The UI draws stills and frames around the hole, so its 120 Hz budget does not carry video. Where no separate layer is available for a kind, it shows a still with a play mark instead.

Everything else follows OPTIONS.md.
