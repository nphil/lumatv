# Home layout payload

`home.xml` is bind-mounted over the stock Home layout inside the Home app's
read-only asset directory:

```
/usr/palm/applications/com.webos.app.home/data/flutter_assets/assets/home.xml
```

The bind covers the whole `assets` directory, so Luma ships a complete
copy of the stock tree with this file (plus the banner images and `i18n/en.json`)
overlaid on top. See `tools/lumatv` for how that copy is built.

## What the layout language can and cannot do

The parser lives in `libapp.so` (compiled Dart, `container_xmlfileload.dart` over
`core/utils/xml_parser.dart`), which means the vocabulary is fixed and the set of
usable element ids is closed. Verified against the binary:

- A page is a **vertical flow of full-width rows**, drawn strictly top to bottom
  in file order. Spacing is created by `margin` rows, not by padding.
- `itemWidth` / `itemHeight` / `focusType` / `hasChildren` / `autoFocus` are
  honoured. **`itemX` / `itemY` are not** - they do not appear in the binary at
  all, so nothing can be positioned at an arbitrary coordinate.
- Sizes are logical 4K units (3840x2160); the compositor scales the result to the
  1080p graphics plane. Full width is `3840`.
- The recognised ids are only those the binary knows: `appList`, `globalline`,
  `herobanner`, `container`, `margin`, `quickGuide`, `qcardList`,
  `recommendedShelf`, `globalactionbuttons`, `editAppList`, `localKeyGuide`,
  `recentApps`, `seniorQcard`, `optionsPopup`, `transparentArea`. An unknown id
  has no widget factory behind it and renders nothing.

## Shipped layout vs stock

Stock (`webOS 10.2.1`, `HE_DTV_W24G_AFABATAA`) in file order, then ours:

| Row | Stock | Luma | Why |
|---|---|---|---|
| `margin` (top) | 3840x45 | 3840x120 | Pushes the rail and hero down clear of the screen edge |
| `container` | 3840x900 | 3840x1520 | Grown so the hero region fills the space the shelves used to occupy |
| `globalline` (in container) | 300x900 | 300x1520 | Matches the taller container; this is the left icon rail |
| `herobanner` (in container) | 3492x900 | 3492x1520 | Matches the taller container; the glass wallpaper shows through it |
| `qcardList` | 3840x180 | removed | Q-card tiles deleted; this row was pure clutter |
| `margin` | 3840x72 | 3840x60 | Retuned for the new stack |
| `appList` | 3840x248 | 3840x248 | Unchanged - the app row is the one thing kept as-is |
| `margin` | 3840x48 | 3840x212 | Absorbs the height freed by the deleted rows so the app row is not clipped |
| `recommendedShelf` | 3798x531 | removed | The single biggest win: this strip is network-loaded from LG's servers on every Home open and was the main cause of stutter and delayed paint |
| `quickGuide` | 0x0 | 0x0 | Already zero-sized in stock; kept for parity |

Net height check: the removed rows (180 + 531) and the margin changes balance so
the rows still sum to the full page height. Getting this wrong clips the app row
off the bottom of the screen, which is exactly what happened during development
before the margins were retuned.

## Notes

- **Order is significant.** `globalline` precedes `herobanner` inside
  `container`, which is what puts the icon rail on the left. Swapping them moves
  the rail to the right edge.
- **Zeroing is safer than deleting** for ids the app still expects to find:
  `quickGuide` is kept at 0x0 rather than removed. Rows that are genuinely
  optional (`qcardList`, `recommendedShelf`) can be dropped outright, but
  deleting the wrong element black-screens Home.
- The greeting and clock strings are **not** set here. They come from the
  compositor QML, which draws its own widgets above the Home card. `en.json.sed`
  only blanks LG's hero strings so they do not collide with those widgets.
