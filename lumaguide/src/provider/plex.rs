//! The guide of the owner's Plex server: the Gracenote listings Plex's DVR keeps for the antenna lineup of a zip code.
//!
//! Every request goes to the server on the home network (plain http, its address and token typed by the owner); the server fetches
//! from Plex's cloud itself and keeps a week of it. Shapes read from a Plex Media Server with an HDHomeRun DVR on the lineup of 30311
//! (2026-11-05):
//!
//! - `GET /livetv/epg/countries/usa/tv.plex.providers.epg.cloud/lineups?postalCode=30311` answers `MediaContainer.Lineup[]` of
//!   `{uuid: "lineup://tv.plex.providers.epg.cloud/<id>#<title>", title, lineupType}`; `lineupType` 0 is the antenna's (1 cable,
//!   2 satellite).
//! - `GET /livetv/dvrs` answers `MediaContainer.Dvr[]` of `{lineup: <uuid>, epgIdentifier: "tv.plex.providers.epg.cloud:5"}`: the
//!   server has a guide only for a lineup one of its DVRs uses.
//! - `GET /<epgIdentifier>/lineups/dvr/channels` answers `MediaContainer.Channel[]` of `{id, vcn: "2.1", callSign: "WSBDT", title}`:
//!   the channels the DVR carries, the ones its grid has.
//! - `GET /<epgIdentifier>/grid?type=1,4&endsAt>=<s>&beginsAt<=<s>` answers `MediaContainer{totalSize, offset, Metadata[]}` of
//!   `{guid, type: "episode"|"movie", title, grandparentTitle, summary, contentRating, Media[{beginsAt, endsAt, channelIdentifier}]}`,
//!   paged by `X-Plex-Container-Start`/`-Size`; a day of 53 channels is some 1,640 airings.
//!
//! The token travels as the `X-Plex-Token` header, never in an address, so an address in a message never holds it.

use super::{Ctx, Error, Lineup, Provider, Source, Station, json_of, keep, send, text};
use crate::model::{Programme, parse_number};
use crate::net::{Http, Request, encode};
use serde_json::Value;

/// Airings asked for in one page.
const PAGE: usize = 1000;
/// What the grid's answer may weigh: a page of a thousand airings is about 1.1 MB.
const MAX_PAGE_BYTES: usize = 6 << 20;
/// Fields and elements of the grid's airings the guide does not read (images, people, sort titles): half of the answer.
const LEAN: &str = "excludeElements=Genre,Image,Role,Director,Writer,Channel,Guid&excludeFields=art,thumb,grandparentArt,grandparentThumb,parentThumb,ratingKey,key,grandparentKey,parentKey,grandparentRatingKey,parentRatingKey,parentGuid,grandparentGuid,titleSort,addedAt,originallyAvailableAt,year,index,parentIndex,parentTitle,skipParent,onAir";

pub struct Plex {
    /// `http://192.168.1.69:32400`.
    base: String,
    token: String,
    /// The DVR's guide, once found.
    epg: Option<String>,
}

impl Plex {
    /// A server at `address` (`192.168.1.69:32400`, or with `http://`).
    pub fn new(address: &str, token: &str) -> Self {
        let address = address.trim().trim_end_matches('/');
        let base = if address.starts_with("http://") || address.starts_with("https://") {
            address.to_owned()
        } else {
            format!("http://{address}")
        };
        Self {
            base,
            token: token.trim().to_owned(),
            epg: None,
        }
    }

    fn get(&self, net: &dyn Http, path: &str, max_bytes: usize) -> Result<Value, Error> {
        let url = format!("{}{path}", self.base);
        let mut request = Request::get(&url)
            .header("Accept", "application/json")
            .header("X-Plex-Token", &self.token);
        request.max_bytes = max_bytes;
        let answer = send(net, &request)?;
        if answer.status == 401 {
            return Err(Error::Credentials(
                "the Plex server turned the token down".into(),
            ));
        }
        json_of(answer, "the Plex server")
    }

    /// The guide of the DVR on `lineup` (or, when it was found before, that one).
    fn epg(&mut self, net: &dyn Http, lineup: &Lineup) -> Result<String, Error> {
        if let Some(epg) = &self.epg {
            return Ok(epg.clone());
        }
        let dvrs = self.get(net, "/livetv/dvrs", 1 << 20)?;
        let found = dvrs["MediaContainer"]["Dvr"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|d| lineup_id(text(d, "lineup")) == lineup.id)
            .map(|d| text(d, "epgIdentifier").to_owned())
            .filter(|e| {
                !e.is_empty()
                    && e.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-' | b'_')
                    })
            });
        let epg = found.ok_or_else(|| {
            Error::Nothing(format!(
                "the Plex server has no DVR on {}: set up Live TV & DVR in Plex with it",
                lineup.name
            ))
        })?;
        self.epg = Some(epg.clone());
        Ok(epg)
    }
}

/// A lineup's uuid without the title Plex puts after `#`.
fn lineup_id(uuid: &str) -> &str {
    uuid.split('#').next().unwrap_or(uuid)
}

/// The programmes of a grid answer: one for each airing (`Media`) of each item.
fn parse_grid(page: &Value) -> Vec<Programme> {
    let mut out = Vec::new();
    for item in page["MediaContainer"]["Metadata"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let episode = text(item, "type") == "episode";
        let (title, subtitle) = match (episode, text(item, "grandparentTitle"), text(item, "title"))
        {
            (true, show, episode) if !show.is_empty() => {
                (show, if episode == show { "" } else { episode })
            }
            (_, _, title) => (title, ""),
        };
        let id = match text(item, "guid") {
            "" => text(item, "ratingKey"),
            guid => guid,
        };
        for airing in item["Media"].as_array().into_iter().flatten() {
            let (Some(begins), Some(ends)) =
                (airing["beginsAt"].as_i64(), airing["endsAt"].as_i64())
            else {
                continue;
            };
            out.push(Programme {
                id: id.to_owned(),
                channel: text(airing, "channelIdentifier").to_owned(),
                title: title.to_owned(),
                subtitle: subtitle.to_owned(),
                text: text(item, "summary").to_owned(),
                start: begins * 1000,
                end: ends * 1000,
                rating: text(item, "contentRating").to_owned(),
                captions: false,
                described: false,
            });
        }
    }
    out
}

impl Provider for Plex {
    fn source(&self) -> Source {
        Source::Plex
    }

    fn lineups(&mut self, cx: Ctx<'_>, zip: &str) -> Result<Vec<Lineup>, Error> {
        let answer = self.get(
            cx.net,
            &format!(
                "/livetv/epg/countries/usa/tv.plex.providers.epg.cloud/lineups?postalCode={}",
                encode(zip)
            ),
            1 << 20,
        )?;
        Ok(answer["MediaContainer"]["Lineup"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| Lineup {
                id: lineup_id(text(l, "uuid")).to_owned(),
                name: text(l, "title").to_owned(),
                antenna: l["lineupType"].as_i64() == Some(0),
            })
            .filter(|l| !l.id.is_empty())
            .collect())
    }

    fn stations(&mut self, cx: Ctx<'_>, lineup: &Lineup) -> Result<Vec<Station>, Error> {
        self.epg = None;
        let epg = self.epg(cx.net, lineup)?;
        let answer = self.get(cx.net, &format!("/{epg}/lineups/dvr/channels"), 2 << 20)?;
        Ok(answer["MediaContainer"]["Channel"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Station {
                key: text(c, "id").to_owned(),
                number: parse_number(text(c, "vcn")),
                callsign: text(c, "callSign").to_owned(),
                name: text(c, "title").to_owned(),
                rf: None,
            })
            .filter(|s| !s.key.is_empty())
            .collect())
    }

    fn airings(
        &mut self,
        cx: Ctx<'_>,
        lineup: &Lineup,
        stations: &[&Station],
        from: i64,
        to: i64,
    ) -> Result<Vec<Programme>, Error> {
        let epg = self.epg(cx.net, lineup)?;
        let mut out = Vec::new();
        let mut offset = 0;
        loop {
            let path = format!(
                "/{epg}/grid?type=1,4&sort=beginsAt&endsAt%3E={}&beginsAt%3C={}&X-Plex-Container-Start={offset}&X-Plex-Container-Size={PAGE}&{LEAN}",
                from / 1000,
                (to - 1) / 1000
            );
            let page = self.get(cx.net, &path, MAX_PAGE_BYTES)?;
            let got = page["MediaContainer"]["Metadata"]
                .as_array()
                .map_or(0, Vec::len);
            out.extend(parse_grid(&page));
            let total = page["MediaContainer"]["totalSize"]
                .as_u64()
                .map_or(0, |t| t as usize);
            offset += got;
            if got == 0 || offset >= total {
                break;
            }
        }
        Ok(keep(out, stations, from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::fake::{Net, cx};
    use crate::time::parse_utc;

    const DVRS: &str = include_str!("../../fixtures/plex/dvrs.json");
    const LINEUPS: &str = include_str!("../../fixtures/plex/lineups.json");
    const CHANNELS: &str = include_str!("../../fixtures/plex/channels.json");
    const GRID: &str = include_str!("../../fixtures/plex/grid.json");

    fn server() -> Net {
        Net::new()
            .on(&["/lineups?postalCode=30311"], 200, LINEUPS)
            .on(&["/livetv/dvrs"], 200, DVRS)
            .on(
                &["/tv.plex.providers.epg.cloud:5/lineups/dvr/channels"],
                200,
                CHANNELS,
            )
            .on(&["/tv.plex.providers.epg.cloud:5/grid?"], 200, GRID)
    }

    #[test]
    fn the_antenna_lineup_of_the_zip_code_is_the_one_plex_types_zero() {
        let net = server();
        let lineups = Plex::new("192.168.1.69:32400", "tk")
            .lineups(cx(&net), "30311")
            .unwrap();
        assert_eq!(lineups.len(), 6);
        let antenna: Vec<&Lineup> = lineups.iter().filter(|l| l.antenna).collect();
        assert_eq!(antenna.len(), 1);
        assert_eq!(
            antenna[0].id,
            "lineup://tv.plex.providers.epg.cloud/5fc76c12598c41002dbe1231"
        );
        assert_eq!(antenna[0].name, "Local Broadcast Listings (156 channels)");
        let seen = net.seen.lock();
        assert_eq!(
            seen[0].url,
            "http://192.168.1.69:32400/livetv/epg/countries/usa/tv.plex.providers.epg.cloud/lineups?postalCode=30311"
        );
        assert!(
            seen[0]
                .headers
                .contains(&("X-Plex-Token".into(), "tk".into()))
        );
        assert!(!seen[0].url.contains("tk"));
    }

    #[test]
    fn the_stations_are_the_channels_of_the_dvr_on_that_lineup_and_their_airings_come_from_its_grid()
     {
        let net = server();
        let mut plex = Plex::new("http://192.168.1.69:32400/", "tk");
        let lineup = plex
            .lineups(cx(&net), "30311")
            .unwrap()
            .into_iter()
            .find(|l| l.antenna)
            .unwrap();
        let stations = plex.stations(cx(&net), &lineup).unwrap();
        assert_eq!(stations.len(), 5);
        assert_eq!(stations[0].number, Some((2, 1)));
        assert_eq!(stations[0].callsign, "WSBDT");
        assert_eq!(stations[3].number, Some((11, 11)));

        let wanted: Vec<&Station> = stations
            .iter()
            .filter(|s| matches!(s.number, Some((2, _))))
            .collect();
        let from = parse_utc("2026-11-05T18:00:00Z").unwrap();
        let airings = plex
            .airings(cx(&net), &lineup, &wanted, from, from + 3 * 3_600_000)
            .unwrap();
        // the 5.1 and the movie on 5.2 are not wanted
        assert_eq!(airings.len(), 6);
        let gma = airings
            .iter()
            .find(|p| p.channel == stations[0].key)
            .unwrap();
        assert_eq!(
            (
                gma.title.as_str(),
                gma.subtitle.as_str(),
                gma.rating.as_str()
            ),
            ("GMA3", "", "TV-PG")
        );
        assert_eq!((gma.start, gma.end), (from, from + 3_600_000));
        assert_eq!(gma.channel, stations[0].key);
        let heat = airings
            .iter()
            .find(|p| p.channel == stations[1].key)
            .unwrap();
        assert_eq!(
            (heat.title.as_str(), heat.subtitle.as_str()),
            ("In the Heat of the Night", "Citizen Trundel")
        );
        let grid = net
            .urls()
            .into_iter()
            .find(|u| u.contains("/grid?"))
            .unwrap();
        assert!(
            grid.contains("endsAt%3E=1793901600&beginsAt%3C=1793912399&X-Plex-Container-Start=0"),
            "{grid}"
        );
    }

    #[test]
    fn a_movie_is_its_own_title() {
        let all = parse_grid(&serde_json::from_str(GRID).unwrap());
        let movie = all
            .iter()
            .find(|p| p.id.starts_with("plex://movie/"))
            .unwrap();
        assert_eq!(
            (movie.title.as_str(), movie.subtitle.as_str()),
            ("The Underworld Story", "")
        );
        assert!(movie.text.starts_with("A newsman"));
    }

    #[test]
    fn a_server_with_no_dvr_on_the_lineup_says_so_and_a_refused_token_is_the_credentials() {
        let net = Net::new().on(&["/livetv/dvrs"], 200, r#"{"MediaContainer":{"size":0}}"#);
        let lineup = Lineup {
            id: "lineup://tv.plex.providers.epg.cloud/x".into(),
            name: "Local Broadcast Listings".into(),
            antenna: true,
        };
        let err = Plex::new("192.168.1.69:32400", "tk")
            .stations(cx(&net), &lineup)
            .unwrap_err();
        assert!(matches!(err, Error::Nothing(_)), "{err:?}");
        let net = Net::new().on(&["/livetv/dvrs"], 401, "");
        let err = Plex::new("192.168.1.69:32400", "bad")
            .stations(cx(&net), &lineup)
            .unwrap_err();
        assert!(matches!(err, Error::Credentials(_)), "{err:?}");
    }
}
