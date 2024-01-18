use anyhow::anyhow;
use serde_json::Value;
use url::Url;

#[derive(Debug, Eq, PartialEq)]
pub struct AudioFileData {
    pub artist: String,
    pub title: String,
}

pub async fn get_musicbrainz_id_for_audio_data(
    audio_file_data: AudioFileData,
) -> anyhow::Result<String> {
    let request_url: Url = Url::parse_with_params(
        "https://api.listenbrainz.org/1/metadata/lookup/",
        &[
            ("artist_name", audio_file_data.artist.clone()),
            ("recording_name", audio_file_data.title.clone()),
        ],
    )?;
    let text = reqwest::get(request_url)
        .await?
        .error_for_status()?
        .text()
        .await?;
    let result = serde_json::from_str::<Value>(text.as_str())?;
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

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_get_recording_mbid_1() {
        let test = AudioFileData {
            artist: "Ed Sheeran".parse().unwrap(),
            title: "Perfect".parse().unwrap(),
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
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
        assert_eq!(result, "");
    }

    #[test]
    #[should_panic]
    fn test_get_recording_mbid_fail_1() {
        let test = AudioFileData {
            artist: "Ed Sheeran".parse().unwrap(),
            title: "Asdjkhfgds".parse().unwrap(),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { get_musicbrainz_id_for_audio_data(test).await.unwrap() });
    }
}
