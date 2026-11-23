# LumaGame UI rework: chosen options

Chosen by the owner on 2026-11-23 from `OPTIONS.md` (mocks in `mock/`).

| Screen | Option | Build notes |
|---|---|---|
| Home | A · Spotlight | Art change uses the dip (fade to stage, swap, fade in), never two full-screen art layers at once. Down opens the game's card strip. |
| Library | B · Panel | Filter panel on the left, search first. |
| Game page | C · Glance | One screen, Play first, four cards (Activity, Community, Media, About). A card opens its section as a layer over a pre-dimmed copy of the art, with no separate scrim layer. |
| Steam and accounts | A · Accounts page | Reached from the quick menu's Accounts and PCs row; game pages show the in-place sign-in row where Steam data would be. |
| Settings | C · Quick panel | The quick menu drawer holds the five most-changed settings, changed in place. Its All settings row opens the full settings in the Rail layout (Settings A), the one place for everything else. |
| Launch and in-game menu | C · Strip | Ring and one line while starting; in a game, a bottom strip that leaves after 5 s. |

Shared by all screens: one quick menu (Menu key) holding Refresh libraries, Accounts and PCs, the quick settings and All settings; Lucent v2 tokens only; 120 fps on the TV is a release gate for every screen and transition.
