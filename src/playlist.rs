use crate::listenbrainz_client::ListenbrainzClient;
use anyhow::{anyhow, Error, Result};
use log::debug;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::str::FromStr;
use url::Url;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct PlaylistSubmissionResponse {
    pub playlist_mbid: Uuid,
}

struct SubmissionPlaylist<'a> {
    name: String,
    song_mbids: &'a [Uuid],
    public: bool,
}

#[derive(Debug, Deserialize)]
struct ValidationResponse {
    code: usize,
    user_name: String,
}

pub struct SimpleExistingPlaylistResponse {
    pub title: String,
    pub identifier: Uuid,
}

pub struct FullExistingPlaylistResponse {
    pub identifier: Uuid,
    pub number_of_tracks: usize,
}

impl SimpleExistingPlaylistResponse {
    pub fn from_json(json: &str) -> Result<Vec<Self>> {
        let data: Value = serde_json::from_str(json)?;
        let mut playlists: Vec<Self> = Vec::new();

        if let Value::Array(individual_playlist) = &data["playlists"] {
            for playlist_data in individual_playlist {
                let identifier = playlist_data["playlist"]["identifier"].as_str().unwrap();
                let title = playlist_data["playlist"]["title"].as_str().unwrap();
                playlists.push(SimpleExistingPlaylistResponse {
                    title: title.to_string(),
                    identifier: Uuid::from_str(
                        identifier
                            .rsplit('/')
                            .collect::<Vec<&str>>()
                            .first()
                            .unwrap(),
                    )
                    .expect("Could not convert to valid UUID"),
                });
            }
        }
        Ok(playlists)
    }
}

impl FullExistingPlaylistResponse {
    pub fn from_json(json: &str) -> Result<Self> {
        let data: Value = serde_json::from_str(json)?;

        let identifier = data["playlist"]["identifier"].as_str().unwrap();
        let number_of_tracks = data["playlist"]["track"].as_array().unwrap().len();
        Ok(FullExistingPlaylistResponse {
            identifier: Uuid::from_str(
                identifier
                    .rsplit('/')
                    .collect::<Vec<&str>>()
                    .first()
                    .unwrap(),
            )
            .expect("Could not convert to valid UUID"),
            number_of_tracks,
        })
    }
    pub async fn convert_simple_playlist_response_to_full(
        listenbrainz_client: &mut ListenbrainzClient,
        simple_playlist: &SimpleExistingPlaylistResponse,
    ) -> Result<Self> {
        get_full_specific_playlist(listenbrainz_client, &simple_playlist.identifier).await
    }
}
impl Serialize for SubmissionPlaylist<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut outer_map = HashMap::new();

        let mut playlist_map = Map::new();
        playlist_map.insert("title".to_string(), Value::String(self.name.clone()));

        let tracks: Vec<Value> = self
            .song_mbids
            .iter()
            .map(|mbid| {
                let mut song_map = Map::new();
                let mut mbid_url = mbid.clone().to_string();
                mbid_url.insert_str(0, "https://musicbrainz.org/recording/");
                song_map.insert("identifier".to_string(), Value::String(mbid_url));
                Value::Object(song_map)
            })
            .collect();
        playlist_map.insert("track".to_string(), Value::Array(tracks));

        let mut extension_map = Map::new();
        let mut musicbrainz_map = Map::new();

        musicbrainz_map.insert("public".to_string(), Value::Bool(self.public));
        extension_map.insert(
            "https://musicbrainz.org/doc/jspf#playlist".to_string(),
            Value::Object(musicbrainz_map),
        );
        playlist_map.insert("extension".to_string(), Value::Object(extension_map));

        outer_map.insert("playlist".to_string(), Value::Object(playlist_map));

        outer_map.serialize(serializer)
    }
}

pub async fn submit_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    mbid_vec: &Vec<Uuid>,
    playlist_name: String,
    public_playlist: bool,
) -> Result<PlaylistSubmissionResponse> {
    let data = SubmissionPlaylist {
        name: playlist_name,
        public: public_playlist,
        song_mbids: mbid_vec,
    };
    let response = listenbrainz_client
        .take_request_builder(
            listenbrainz_client
                .request_client
                .post("https://api.listenbrainz.org/1/playlist/create")
                .json(&data),
        )
        .await?;
    let playlist_id = response.json::<PlaylistSubmissionResponse>().await?;
    Ok(playlist_id)
}

pub async fn get_current_user(listenbrainz_client: &mut ListenbrainzClient) -> Result<String> {
    let response = listenbrainz_client
        .take_request_builder(
            listenbrainz_client
                .request_client
                .get("https://api.listenbrainz.org/1/validate-token"),
        )
        .await?;
    let response_text = response.text().await?;
    let response: ValidationResponse = serde_json::from_str(response_text.as_str())?;
    match response.code {
        200 => Ok(response.user_name),
        _ => Err(anyhow!("Response was {}", response.code)),
    }
}

pub async fn get_current_playlists(
    listenbrainz_client: &mut ListenbrainzClient,
    user_name: &String,
) -> Result<Vec<SimpleExistingPlaylistResponse>> {
    let url = Url::parse_with_params(
        &format!("https://api.listenbrainz.org/1/user/{user_name}/playlists"),
        [("count", u32::MAX.to_string())],
    )?;
    let response = listenbrainz_client
        .take_request_builder(listenbrainz_client.request_client.get(url))
        .await;
    let response_text = response?.text().await?;
    let playlist_objects = SimpleExistingPlaylistResponse::from_json(response_text.as_str())?;
    Ok(playlist_objects)
}

async fn get_full_specific_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    playlist_id: &Uuid,
) -> Result<FullExistingPlaylistResponse> {
    let url = Url::parse(&format!(
        "https://api.listenbrainz.org/1/playlist/{playlist_id}"
    ))?;
    let response = listenbrainz_client
        .take_request_builder(listenbrainz_client.request_client.get(url))
        .await;
    let response_text = response?.text().await?;
    let playlist_objects = FullExistingPlaylistResponse::from_json(response_text.as_str())?;
    Ok(playlist_objects)
}

pub async fn delete_items_from_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    playlist_id: &Uuid,
    start_index: usize,
    count_to_remove: usize,
) -> Result<()> {
    let url = Url::parse(&format!(
        "https://api.listenbrainz.org/1/playlist/{playlist_id}/item/delete",
    ))?;
    debug!("Deleting tracks from playlist with URL '{url}'");
    let data = HashMap::from([("index", start_index), ("count", count_to_remove)]);
    let response = listenbrainz_client
        .take_request_builder(listenbrainz_client.request_client.post(url).json(&data))
        .await;
    let response = response?.status();
    match_error_from_playlist_change(response)
}

pub async fn mass_add_to_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    playlist_id: &Uuid,
    track_mbids: &[Uuid],
) -> Result<()> {
    for chunk in track_mbids.chunks(100) {
        add_items_to_playlist(listenbrainz_client, playlist_id, chunk).await?;
    }
    Ok(())
}

pub async fn add_items_to_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    playlist_id: &Uuid,
    track_mbids: &[Uuid],
) -> Result<()> {
    let url = Url::parse(&format!(
        "https://api.listenbrainz.org/1/playlist/{playlist_id}/item/add/0",
    ))?;
    debug!("Inserting tracks to playlist with URL '{}'", &url);
    let data = SubmissionPlaylist {
        name: "addition".to_string(),
        public: false,
        song_mbids: track_mbids,
    };
    debug!("{:?}", serde_json::to_string(&data));
    let response = listenbrainz_client
        .take_request_builder(listenbrainz_client.request_client.post(url).json(&data))
        .await;
    let response = response?.status();
    match_error_from_playlist_change(response)
}

fn match_error_from_playlist_change(response: StatusCode) -> Result<(), Error> {
    match response.as_u16() {
        200 => Ok(()),
        400 => Err(anyhow!("Request was badly formulated (400)")),
        401 => Err(anyhow!("Authorisation failed (401)")),
        403 => Err(anyhow!("Not authorised to delete from this playlist (403)")),
        error => Err(anyhow!("The request returned error code {}", error)),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_test::{assert_ser_tokens, Token};

    #[test]
    fn test_serialise_playlist_no_tracks() {
        let test = SubmissionPlaylist {
            name: "Example".to_string(),
            song_mbids: &Vec::new(),
            public: false,
        };
        assert_ser_tokens(
            &test,
            &[
                Token::Map { len: Some(1) },
                Token::Str("playlist"),
                Token::Map { len: Some(3) },
                Token::Str("extension"),
                Token::Map { len: Some(1) },
                Token::Str("https://musicbrainz.org/doc/jspf#playlist"),
                Token::Map { len: Some(1) },
                Token::Str("public"),
                Token::Bool(false),
                Token::MapEnd,
                Token::MapEnd,
                Token::Str("title"),
                Token::Str("Example"),
                Token::Str("track"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::MapEnd,
                Token::MapEnd,
            ],
        );
    }

    #[test]
    fn test_serialise_playlist_one_track() {
        let test = SubmissionPlaylist {
            name: "Example".to_string(),
            song_mbids: &vec![Uuid::from_str("36855a5c-abcb-4740-9154-361af8c11ee1").unwrap()],
            public: false,
        };
        assert_ser_tokens(
            &test,
            &[
                Token::Map { len: Some(1) },
                Token::Str("playlist"),
                Token::Map { len: Some(3) },
                Token::Str("extension"),
                Token::Map { len: Some(1) },
                Token::Str("https://musicbrainz.org/doc/jspf#playlist"),
                Token::Map { len: Some(1) },
                Token::Str("public"),
                Token::Bool(false),
                Token::MapEnd,
                Token::MapEnd,
                Token::Str("title"),
                Token::Str("Example"),
                Token::Str("track"),
                Token::Seq { len: Some(1) },
                Token::Map { len: Some(1) },
                Token::String("identifier"),
                Token::String(
                    "https://musicbrainz.org/recording/36855a5c-abcb-4740-9154-361af8c11ee1",
                ),
                Token::MapEnd,
                Token::SeqEnd,
                Token::MapEnd,
                Token::MapEnd,
            ],
        );
    }

    #[test]
    fn test_serialise_playlist_two_tracks() {
        let test = SubmissionPlaylist {
            name: "Example".to_string(),
            song_mbids: &vec![
                Uuid::from_str("36855a5c-abcb-4740-9154-361af8c11ee1").unwrap(),
                Uuid::from_str("00066722-b23a-48e5-82e4-0470c82a2705").unwrap(),
            ],
            public: false,
        };
        assert_ser_tokens(
            &test,
            &[
                Token::Map { len: Some(1) },
                Token::Str("playlist"),
                Token::Map { len: Some(3) },
                Token::Str("extension"),
                Token::Map { len: Some(1) },
                Token::Str("https://musicbrainz.org/doc/jspf#playlist"),
                Token::Map { len: Some(1) },
                Token::Str("public"),
                Token::Bool(false),
                Token::MapEnd,
                Token::MapEnd,
                Token::Str("title"),
                Token::Str("Example"),
                Token::Str("track"),
                Token::Seq { len: Some(2) },
                Token::Map { len: Some(1) },
                Token::String("identifier"),
                Token::String(
                    "https://musicbrainz.org/recording/36855a5c-abcb-4740-9154-361af8c11ee1",
                ),
                Token::MapEnd,
                Token::Map { len: Some(1) },
                Token::String("identifier"),
                Token::String(
                    "https://musicbrainz.org/recording/00066722-b23a-48e5-82e4-0470c82a2705",
                ),
                Token::MapEnd,
                Token::SeqEnd,
                Token::MapEnd,
                Token::MapEnd,
            ],
        );
    }
}
