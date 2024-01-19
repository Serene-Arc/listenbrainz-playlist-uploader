use anyhow::{anyhow, Result};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::HashMap;
use url::Url;

#[derive(Deserialize)]
pub struct PlaylistSubmissionResponse {
    pub playlist_mbid: String,
}

struct SubmissionPlaylist<'a> {
    name: String,
    song_mbids: &'a Vec<String>,
    public: bool,
}

#[derive(Debug, Deserialize)]
struct ValidationResponse {
    code: usize,
    valid: bool,
    user_name: String,
}

#[derive(Deserialize)]
pub struct ExistingPlaylistResponse {
    pub title: String,
    pub identifier: String,
}

impl ExistingPlaylistResponse {
    pub fn from_json(json: &str) -> Result<Vec<Self>> {
        let data: Value = serde_json::from_str(json)?;
        let mut playlists: Vec<Self> = Vec::new();

        if let Value::Array(playlists_data) = &data["playlists"] {
            for playlist_data in playlists_data {
                let identifier = playlist_data["playlist"]["identifier"].as_str().unwrap();
                let title = playlist_data["playlist"]["title"].as_str().unwrap();
                playlists.push(ExistingPlaylistResponse {
                    title: title.to_string(),
                    identifier: identifier.to_string(),
                });
            }
        }

        Ok(playlists)
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
                let mut mbid_url = mbid.clone();
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
    user_token: &String,
    mbid_vec: &Vec<String>,
    playlist_name: String,
    public_playlist: bool,
) -> Result<PlaylistSubmissionResponse> {
    let client = reqwest::Client::new();
    let data = SubmissionPlaylist {
        name: playlist_name,
        public: public_playlist,
        song_mbids: mbid_vec,
    };
    let response = client
        .post("https://api.listenbrainz.org/1/playlist/create")
        .header(AUTHORIZATION, format!("Token {}", user_token))
        .json(&data)
        .send()
        .await?;
    let playlist_id = response.json::<PlaylistSubmissionResponse>().await?;
    Ok(playlist_id)
}

pub async fn get_current_user(user_token: &String) -> Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.listenbrainz.org/1/validate-token")
        .header(AUTHORIZATION, format!("Token {}", user_token))
        .send()
        .await?;
    let test = response.text().await?;
    let response: ValidationResponse = serde_json::from_str(test.as_str())?;
    match response.code {
        200 => return Ok(response.user_name),
        _ => Err(anyhow!("Response was {}", response.code)),
    }
}

pub async fn get_current_playlists(
    token: &String,
    user_name: &String,
) -> Result<Vec<ExistingPlaylistResponse>> {
    let url = Url::parse(&*format!(
        "https://api.listenbrainz.org/1/user/{}/playlists",
        user_name
    ))?;
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .header(AUTHORIZATION, format!("Token {}", token))
        .send()
        .await;
    let response_text = response?.text().await?;
    let playlist_objects = ExistingPlaylistResponse::from_json(response_text.as_str())?;
    Ok(playlist_objects)
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
            song_mbids: &vec!["test".to_string()],
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
                Token::String("https://musicbrainz.org/recording/test"),
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
            song_mbids: &vec!["test1".to_string(), "test2".to_string()],
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
                Token::String("https://musicbrainz.org/recording/test1"),
                Token::MapEnd,
                Token::Map { len: Some(1) },
                Token::String("identifier"),
                Token::String("https://musicbrainz.org/recording/test2"),
                Token::MapEnd,
                Token::SeqEnd,
                Token::MapEnd,
                Token::MapEnd,
            ],
        );
    }
}
