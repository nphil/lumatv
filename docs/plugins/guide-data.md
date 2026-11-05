# Guide data

The Guide data plugin fills the Live TV guide and the channel banner with online listings for a US zip code, laid over the TV's own
broadcast listings. The broadcast guide is often a few hours to a day, with gaps and few descriptions; the online sources carry a week of
Gracenote listings. It is off until the owner switches it on in Settings › Inputs & devices › Guide data, which asks first in the
plugin's own words. Like the Live TV Tuner it is a plugin of the helper's own (`helper/app/src/guidedata.rs`).

## Sources

The page picks any of three, in the order they are trusted. A picked source that is not set up (Plex without an address or token,
Schedules Direct without its account) is passed over and the next one fetched; its line on the page says it is skipped and what to
type, and the page's status is the other sources' until every picked source is unset.

| Source | Needs | Reaches | Tradeoff |
|---|---|---|---|
| Plex | a Plex server whose Live TV & DVR uses the zip code's antenna lineup; its address and token, or the Photos plugin's | the server, on the home network, plain http | Plex's own endpoints, unofficial: a server update can change them. Free with the DVR the owner already has. |
| Schedules Direct | an account (schedulesdirect.org, about $35 a year): user name and password | `json.schedulesdirect.org`, https | Official and documented, made for this use; paid. Built against the API's documentation; not yet tried against the live service. |
| Free listings | nothing | `tvlistings.gracenote.com`, https | No account and no cost, but unofficial and fragile: the grid behind the site's own page, answered only to a browser's `User-Agent`, with no published terms for this use. |

## Zip code and channels

The zip code picks each source's antenna lineup. A source's stations are matched to the TV's channels by virtual number (`2.1`), then
by call sign and minor number, then by RF channel and minor number; a channel no source lists keeps the TV's own listings. For 30311
and the set's 57 channels (2026-11-05), Plex and the free listings each match 53; neither lists 35.2, 35.3, 36.4 or 66.1.

## Merge

The first source that has a programme for a time wins it, a later one fills only what the ones before leave empty, and the TV's broadcast
listings fill the rest. A programme that overlaps one already kept is left out whole rather than cut.

## Fetching

A week is kept in the helper's data directory (`guide/listings`, one document per source per UTC day). A source is asked at most every
6 hours: today and tomorrow are fetched again, the later days once a day. A failed request waits 30 minutes; "Refresh now" still waits an
hour after the last try. A week from Plex was 16 requests and 11.7 MB (3.5 MB kept); from the free listings 30 requests and 19.5 MB
(3.2 MB kept).

The plugin reaches only the picked services: each one's host over https, and the Plex server only at an address of the home network.

## Where the listings go

- The guide and the banner ask `GET /v1/guide/programmes?channel=<id>&from=<ms>&to=<ms>` (`{ok, on, programmes}`) alongside LG's own
  listings and lay the answer over them.
- The Live TV Tuner's `/xmltv.xml` serves the merged guide while the plugin is on.

## Secrets

The Plex token and the Schedules Direct password are secret fields: Settings hands each to the plugin once
(`POST /v1/plugins/guide-data/secret`), the settings never hold it, and the plugin keeps it in `guide/secrets.json` (mode 0600).
Schedules Direct is sent only the password's SHA-1. While the page has no Plex address, the Photos plugin's server and token are used.

## Engine

The engine is the `lumaguide` crate, with no LumaTV host in it: a `Provider` for each source, lineup discovery by zip code, the channel
map, the cache and the merge, giving listings and XMLTV. The host lends it the network (`net::Http`) and the storage (`store::Store`).
