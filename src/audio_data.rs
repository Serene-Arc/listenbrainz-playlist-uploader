use anyhow::{anyhow, Result};
use audiotags::Tag;
use cached::proc_macro::cached;
use musicbrainz_rs::entity::artist::{Artist, ArtistSearchQuery};
use musicbrainz_rs::entity::recording::{Recording, RecordingSearchQuery};
use musicbrainz_rs::Search;
use std::path::PathBuf;

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

pub async fn get_musicbrainz_id_for_audio_data(audio_file_data: AudioFileData) -> Result<String> {
    let artist_identifier = get_artist_mbid(audio_file_data.artist.clone()).await;

    let query = construct_song_search_query(&audio_file_data, artist_identifier);
    let result = Recording::search(query).execute().await?;
    let all_mbids: Vec<_> = result.entities.iter().map(|e| e.id.clone()).collect();
    if all_mbids.len() <= 0 {
        return Err(anyhow!(
            "No matches found for the given song tags: {:?}",
            audio_file_data
        ));
    }
    // TODO: something fancy here to choose the best one
    // For now, just return the first one
    Ok(all_mbids.first().expect("Could not get first MBID").clone())
}

fn construct_song_search_query(audio_file_data: &AudioFileData, artist_data: ArtistData) -> String {
    let mut query = RecordingSearchQuery::query_builder();
    match artist_data.mbid {
        None => query.artist(artist_data.artist_tag.as_str()),
        Some(mbid) => query.arid(mbid.as_str()),
    }
    .and()
    .recording(audio_file_data.title.as_str());

    match &audio_file_data.album {
        None => {}
        Some(a) => {
            query.and().release(a.as_str());
        }
    }
    query.build()
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
    let first_mbid = result.entities.first().unwrap().id.clone();
    ArtistData {
        artist_tag: artist_name.clone(),
        mbid: Some(first_mbid),
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
        album: if album.len() > 0 { Some(album) } else { None },
    })
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_get_recording_mbid_1() {
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
    fn test_get_recording_mbid_2() {
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
    fn test_get_recording_mbid_3() {
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
        assert_eq!(result.mbid.unwrap(), "b8a7c51f-362c-4dcb-a259-bc6e0095f0a6")
    }

    #[test]
    fn test_get_artist_mbid_2_non_english_with_alias() {
        let test = "Akihito Okano".to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_artist_mbid(test).await });
        assert_eq!(result.mbid.unwrap(), "0f51ab24-c89a-438e-b3af-2d974fa0654a")
    }
}
