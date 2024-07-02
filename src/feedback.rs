use crate::listenbrainz_client::ListenbrainzClient;
use crate::paginator::ListenbrainzPaginator;
use crate::Feedback;
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::str::FromStr;
use url::Url;
use uuid::Uuid;

#[derive(Deserialize)]
struct FeedbackResponse {
    // It's possible that there are recordings with feedback that have no MBID
    recording_mbid: Option<String>,
}

#[derive(Deserialize)]
struct FeedbackResponseWrapper {
    feedback: Vec<FeedbackResponse>,
    count: usize,
}

pub async fn give_song_feedback_for_mbid(
    listenbrainz_client: &mut ListenbrainzClient,
    mbid: &Uuid,
    feedback: Feedback,
) -> Result<()> {
    let parameters = json!({"recording_mbid": mbid,"score": &(feedback as i8).to_string(),});
    let response = listenbrainz_client
        .take_request_builder(
            listenbrainz_client
                .request_client
                .post("https://api.listenbrainz.org/1/feedback/recording-feedback")
                .json(&parameters),
        )
        .await;
    match response {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub async fn get_existing_feedback(
    listenbrainz_client: &mut ListenbrainzClient,
    user_name: &str,
    feedback: Feedback,
) -> Result<HashSet<Uuid>> {
    let mut all_feedback = HashSet::new();
    for url in ListenbrainzPaginator::new(
        &format!("https://api.listenbrainz.org/1/feedback/user/{user_name}/get-feedback"),
        0,
        1000,
    ) {
        let real_url =
            Url::parse_with_params(url.as_ref(), [("score", (feedback as i8).to_string())])
                .expect("Could not construct url");
        let response = listenbrainz_client
            .take_request_builder(listenbrainz_client.request_client.get(real_url))
            .await?;
        let response_text = response.text().await?;
        let feedback_response: FeedbackResponseWrapper =
            serde_json::from_str(response_text.as_str())?;
        if feedback_response.count == 0 {
            break;
        }
        all_feedback.extend(
            feedback_response
                .feedback
                .into_iter()
                .filter_map(|f| f.recording_mbid)
                .map(|m| Uuid::from_str(m.as_str()))
                .filter_map(|f| f.ok()),
        )
    }
    Ok(all_feedback)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_existing_feedback() {
        let mut test_client = ListenbrainzClient::new("".to_string());
        let result = get_existing_feedback(&mut test_client, "Serene-Arc", Feedback::Love);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { result.await }).unwrap();
        // Magic number for me specifically; I know I have more than 100 favourites
        assert!(result.len() > 100)
    }
}
