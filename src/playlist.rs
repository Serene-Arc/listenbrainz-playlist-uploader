use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct PlaylistResponse {
    pub playlist_mbid: String,
}

struct Playlist<'a> {
    name: String,
    song_mbids: &'a Vec<String>,
    public: bool,
}

impl Serialize for Playlist<'_> {
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
) -> anyhow::Result<PlaylistResponse> {
    let client = reqwest::Client::new();
    let data = Playlist {
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
    let playlist_id = response.json::<PlaylistResponse>().await?;
    Ok(playlist_id)
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_test::{assert_ser_tokens, Token};

    #[test]
    fn test_serialise_playlist_no_tracks() {
        let test = Playlist {
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
                Token::Str("title"),
                Token::Str("Example"),
                Token::Str("track"),
                Token::Seq { len: Some(0) },
                Token::SeqEnd,
                Token::Str("extension"),
                Token::Map { len: Some(1) },
                Token::Str("https://musicbrainz.org/doc/jspf#playlist"),
                Token::Map { len: Some(1) },
                Token::Str("public"),
                Token::Bool(true),
                Token::MapEnd,
                Token::MapEnd,
                Token::MapEnd,
                Token::MapEnd,
            ],
        );
    }
}
