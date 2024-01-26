use crate::listenbrainz_client::ListenbrainzClient;
use anyhow::{anyhow, Result};
use audiotags::Tag;
use cached::proc_macro::cached;
use musicbrainz_rs::entity::artist::{Artist, ArtistSearchQuery};
use musicbrainz_rs::Search;
use serde_json::Value;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Eq, PartialEq)]
pub struct AudioFileData {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ArtistData {
    pub artist_tag: String,
    pub mbid: Option<String>,
}

pub async fn get_musicbrainz_id_for_audio_data(
    listenbrainz_client: &mut ListenbrainzClient,
    audio_file_data: AudioFileData,
) -> Result<String> {
    let mut result = make_listenbrainz_lookup_request(
        listenbrainz_client,
        &audio_file_data.title,
        &audio_file_data.artist,
    )
    .await?;

    if result.as_object().unwrap().is_empty() {
        // Attempt to resolve artist and try that, it might be an alias
        let artist = get_artist_mbid(audio_file_data.artist.clone()).await;
        result = make_listenbrainz_lookup_request(
            listenbrainz_client,
            &audio_file_data.title,
            &artist.artist_tag,
        )
        .await?;
    }

    if result.as_object().unwrap().is_empty() {
        return Err(anyhow::anyhow!("Could not resolve {:?}", audio_file_data));
    }

    let out = result
        .get("recording_mbid")
        .ok_or_else(|| anyhow::anyhow!("Could not extract recording MBID from JSON: {:?}", result))?
        .as_str()
        .ok_or_else(|| anyhow!("Could not convert to string"))?;
    Ok(out.to_string())
}

async fn make_listenbrainz_lookup_request(
    listenbrainz_client: &mut ListenbrainzClient,
    title: &String,
    artist: &String,
) -> Result<Value> {
    let request_url: Url = Url::parse_with_params(
        "https://api.listenbrainz.org/1/metadata/lookup/",
        &[("artist_name", artist), ("recording_name", title)],
    )?;
    let result = listenbrainz_client
        .take_request_builder(listenbrainz_client.request_client.get(request_url))
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    Ok(result)
}

#[cached]
async fn get_artist_mbid(artist_name: String) -> ArtistData {
    let query = ArtistSearchQuery::query_builder()
        .artist(artist_name.as_str())
        .build();
    let mut result = Artist::search(query)
        .execute()
        .await
        .expect("Could not make search");

    // If no results found, find an alias instead
    if result.count <= 0 {
        let query = ArtistSearchQuery::query_builder()
            .alias(artist_name.as_str())
            .build();
        result = Artist::search(query)
            .execute()
            .await
            .expect("Could not make search");
    }

    if result.count <= 0 {
        return ArtistData {
            artist_tag: artist_name.clone(),
            mbid: None,
        };
    }

    // TODO: need to do something clever here too to find the best one
    let artist = result.entities.first().unwrap();
    ArtistData {
        artist_tag: artist.name.clone(),
        mbid: Some(artist.id.clone()),
    }
}

pub fn load_tags_from_file_path(playlist_entries: PathBuf) -> Result<AudioFileData> {
    let tags = Tag::new().read_from_path(playlist_entries)?;
    let artist = tags
        .artist()
        .ok_or(anyhow!("Could not read artist"))?
        .parse()?;
    let title = tags
        .title()
        .ok_or(anyhow!("Could not read title"))?
        .parse()?;
    let album = tags
        .album()
        .ok_or(anyhow!("Could not read album"))?
        .title
        .to_string();
    Ok(AudioFileData {
        artist,
        title,
        album: if album.is_empty() { None } else { Some(album) },
    })
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_get_recording_mbid_general_1() {
        let test = AudioFileData {
            artist: "Ed Sheeran".parse().unwrap(),
            title: "Perfect".parse().unwrap(),
            album: Some("Divide".to_string()),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "b84dd2d1-2bf1-4fcc-aadc-6cc39c36ba35");
    }

    #[test]
    fn test_get_recording_mbid_general_2() {
        let test = AudioFileData {
            artist: "Sasha Alex Sloan".parse().unwrap(),
            title: "Dancing with Your Ghost".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "9ae71082-ac47-4b9c-a12b-a67fff75784a");
    }

    #[test]
    fn test_get_recording_mbid_artist_alias() {
        let test = AudioFileData {
            artist: "Akihito Okano".parse().unwrap(),
            title: "光あれ".parse().unwrap(),
            album: Some("光あれ".parse().unwrap()),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "5d93f99e-6663-4e77-97f1-0835f6b96b00");
    }

    #[test]
    fn test_get_recording_mbid_two_artists_and_join() {
        let test = AudioFileData {
            artist: "Ed Sheeran & Beyonce".parse().unwrap(),
            title: "Perfect Duet".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "764f4c40-1c16-44a7-a6e6-b8c426604b57");
    }

    #[test]
    fn test_get_recording_mbid_band_name_with_character() {
        let test = AudioFileData {
            artist: "Florence + the Machine".parse().unwrap(),
            title: "Never Let Me Go".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "589b2eff-e541-475b-bbe7-ca778238e711");
    }

    #[test]
    fn test_get_recording_mbid_two_artist_feat_join() {
        let test = AudioFileData {
            artist: "Justin Bieber feat. Khalid".parse().unwrap(),
            title: "As I Am".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "4f8268ae-8db1-42a7-baca-b1a0b0b879c4");
    }

    #[test]
    fn test_get_recording_mbid_artist_partial_name() {
        let test = AudioFileData {
            artist: "Sasha Sloan".parse().unwrap(),
            title: "Dancing with Your Ghost".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "9ae71082-ac47-4b9c-a12b-a67fff75784a");
    }

    #[test]
    #[should_panic]
    fn test_get_recording_mbid_fail_1() {
        let test = AudioFileData {
            artist: "Ed Sheeran".parse().unwrap(),
            title: "Asdjkhfgds".parse().unwrap(),
            album: None,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
    }

    #[test]
    fn test_get_artist_mbid_1() {
        let test = "Ed Sheeran".to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_artist_mbid(test).await });
        assert_eq!(result.mbid.unwrap(), "b8a7c51f-362c-4dcb-a259-bc6e0095f0a6");
    }

    #[test]
    fn test_get_artist_mbid_2_non_english_with_alias() {
        let test = "Akihito Okano".to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_artist_mbid(test).await });
        assert_eq!(result.mbid.unwrap(), "0f51ab24-c89a-438e-b3af-2d974fa0654a");
    }
}
