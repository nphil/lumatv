# Steam Web API key (no sign-in)

LumaGame's Steam engine can link an account with a Steam Web API key and the account's SteamID instead of the QR sign-in. No session is
created, so nothing on the TV renews or expires, and a password change does not end it. It is read-only.

## What Valve documents

Sources: `partner.steamgames.com/doc/webapi_overview/auth`, `steamcommunity.com/dev/apiterms`, the per-method pages, and Valve's
"Limited User Accounts" FAQ.

- A standard user key is open to everyone with a Steam account and a domain name, made at `https://steamcommunity.com/dev/apikey`
  (sign-in required) after agreeing to the Steam Web API Terms of Use.
- The key travels as the `key` query parameter or the `x-webapi-key` header.
- 401 and 403 mean the key is refused and a retry will not help; 403s carry strict per-IP limits. 429 is rate limiting. 500 and 503 may be
  retried.
- Terms: the licence is personal to the key holder, the key stays confidential and is not shared or transferred, data is fetched only as
  the end user asks and the user is told what is stored, 100,000 calls a day, Valve may change or end access at any time.
- A "limited user account" (one that has not spent the minimum on Steam) lacks Web API access. A limited account cannot make a key.
- Methods used: `IPlayerService/GetOwnedGames/v1` and `GetRecentlyPlayedGames/v1` ("if their owned games/game details are visible to
  you"), `ISteamUser/GetPlayerSummaries/v2`, `ISteamUser/ResolveVanityURL/v1`, `ISteamUserStats/GetPlayerAchievements/v1`.
- Not usable: `GetSingleGamePlaytime` needs a key tied to the app.

## Not documented, to be confirmed on the first real link

1. Whether the key's owner reads their own library and achievements while Game details is private. Developers report yes; Valve says
   only "if visible to you".
2. Whether creating a key asks for a Steam Guard mobile confirmation.
3. Whether a key survives a password change. It belongs to the account, not a session, so probably.
4. The exact answer for a private profile on `GetPlayerAchievements` (expected: 403 with JSON `Profile is not public`; a refused key
   answers 403 with an HTML page, and the engine tells them apart by the body).
5. Whether the `key` query parameter and the `x-webapi-key` header behave alike. The engine uses the query form and never logs a URL;
   the header is the fallback if a proxy or log is ever found to keep URLs.
6. Whether `GetOwnedGames` carries `playtime_2weeks` (the engine asks `GetRecentlyPlayedGames` for it either way).

## What a key covers

Covered: the account's name and picture, owned games with play time and last played, play time of the last two weeks, achievements per
game, and the store data (genres, tags) the engine already reads without a sign-in.

Not covered: friends, favourites and collections (a sign-in surface only), and anything private to the account.

When Steam returns no games, the engine says so and keeps the last listing; the usual cause is Game details set to Private under Steam's
privacy settings. A refused key shows as "Steam did not accept the key" and is not tried again until a new key is entered; the engine
never opens a QR sign-in by itself.

## Owner steps

1. On a phone or PC, sign in at steamcommunity.com and open `https://steamcommunity.com/dev/apikey`.
2. Enter any domain name, tick the terms box and register. Copy the 32-character key.
3. In Steam, Profile, Edit Profile, Privacy Settings: set Game details to Public (may not be needed for the key's owner; see 1).
4. Find the SteamID64: the 17-digit number in the profile address (`steamcommunity.com/profiles/<number>`). A profile address or a custom
   profile name works too.
5. On the TV: Settings, Accounts and PCs, Steam, Use a Web API key. Type the key on the hex keys, then the Steam ID, then Link.

A key entered in LumaGame is kept in the engine's private data directory (`apikey.json`, mode 0600) and removed by Sign out. The key is
as sensitive as a password for the API: regenerate it at the same page if it leaks.
