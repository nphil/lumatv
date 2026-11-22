# Controller button markings

What the two owned pads print on their face buttons, and how LumaGame's hints draw it. Reference photos are kept outside the repo.

## GameSir Cyclone 2

Source: https://gamesir.com/products/gamesir-cyclone2-black (product photos, the manual's layout page) and the E-manual at https://www.gamesir.hk/pages/manuals-gamesir-cyclone2.

| Part | Marking |
| --- | --- |
| Face buttons | Round, glossy black, Xbox layout: Y top, X left, B right, A bottom. Plain white letters, no colour. |
| Letters | Light geometric sans, about 0.46 of the button's diameter tall, stroke about 0.06 of the diameter. |
| Cluster | A small "×" mark between the four buttons. |
| Home | Round, GameSir eagle-head logo inside a red ring. |
| View | Small tilted pill, overlapping-squares icon, "−" printed below. |
| Menu | Small tilted pill, slanted three-line icon, "+" printed below. |
| Others | Share pill in the centre, "M" button below it, back buttons. Their labels were not read. |

The letters are the same in every mode (X-input, DS4, Switch); in DS4 mode the bottom button is still the one the pad reports as South, so A.

## Leadjoy Xeno Plus

Source: https://www.leadjoy.ai/products/leadjoy-xeno-plus (page title "Xeno Plus PC"; the shop's page images, the main and 1000x1000 shots).

| Part | Marking |
| --- | --- |
| Face buttons | Round, glossy black, Xbox layout. Letters printed as white-grey dot matrices, uncoloured. |
| Letters | Round dots, 0.72–0.8 of the dot pitch across; a letter is about 51% of the button's diameter tall. |
| View / Menu | Dotted overlapping squares, dotted "≡" (four lines seen). A dotted rounded-square button sits between them. |
| Home | Round, the Leadjoy swirl logo in dots. |
| Others | "LEADJOY" in purple dots on the top plate, back paddles L4/R4. Sticks and triggers carry purple accents; the letters do not. |

Dot grids as read from the photos (`#` is a dot):

```
A  .#####.   B  ######.   X  ##....##   Y  ##....##
   #######      #######      ###..###      ###..###
   ##...##      ##...##      .######.      .######.
   ##...##      ######.      ..####..      ..####..
   #######      #######      ..####..      ...##...
   #######      ##...##      .######.      ...##...
   ##...##      #######      ###..###      ...##...
   ##...##      ######.      ##....##      ...##...
                                           ...##...
```

## Sources disagree or are thin

- Only one edition of each page was seen: the Cyclone 2 in black (the white one is a colour change) and the Leadjoy "PC" page. Both USB ids in the notes (`054c:09cc` DS4 mode, `4131:8008` Leadjoy vendor interface) are the same hardware as those pages; no variant with other letters turned up.
- The Xeno Plus runs in XInput, Switch, PS4, PS5 and touch modes. Its play-mode USB ids and kernel name were never read, and the only `4131:8008` observation is the vendor-only interface with no input node.
- The manual's layout crop stops before the Cyclone 2's bumpers and triggers, so those labels are unconfirmed. Whether the Xeno Plus Menu icon has three or four lines is unconfirmed (four seen).
- The Cyclone 2 in DS4 mode has the same USB id as a real DualShock 4 v2, so the id cannot identify it.

## How LumaGame draws and picks them

- The glyphs are vector shapes (`rustui/app/src/game/screens/symbols.rs`, `Symbol::Face`): lines for the GameSir letters, dots for the Leadjoy ones. Hints only use A, B and Y.
- The button is the theme's ink with a canvas marking in light mode; in dark mode the canvas with an ink marking and an ink rim, so it stays visible on a dark sheet. No other colour is used, so it follows theme and mode live. On a primary button the disc and marking take the button's ink and fill, as the Xbox glyphs do.
- The PlayStation option draws a generic ✕ ○ △ in the same ink rules; it is not taken from a pad.
- Detection uses the kernel's name for the pad. A name containing "chicken run" or "gamesir" (the Cyclone 2's USB manufacturer is "Guangzhou Chicken Run Network Technology Co., Ltd.") selects the GameSir letters; "leadjoy", "xeno" or "liteemf" selects the Leadjoy ones; anything else stays Xbox. With several pads, the one pressed last decides. Settings › Controllers › Button labels overrides it (Automatic, Xbox, PlayStation, GameSir Cyclone 2, Leadjoy Xeno Plus).
- The kernel names have not been read on the TV; the Cyclone 2's follows from its USB strings, the Xeno Plus's play-mode name is unknown. Until the real names are seen, the setting is the dependable path.
