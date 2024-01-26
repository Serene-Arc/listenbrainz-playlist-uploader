use crate::paginator::ListenbrainzPaginator;
use crate::Feedback;
use anyhow::{Error, Result};
use reqwest::header::AUTHORIZATION;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use url::Url;

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
    user_token: &str,
    mbid: &str,
    feedback: Feedback,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let parameters = json!({"recording_mbid": mbid,"score": &(feedback as i8).to_string(),});
    let response = client
        .post("https://api.listenbrainz.org/1/feedback/recording-feedback")
        .header(AUTHORIZATION, format!("Token {user_token}"))
        .json(&parameters)
        .send()
        .await;
    match response {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::from(e)),
    }
}

pub async fn get_existing_feedback(user_name: &str, feedback: Feedback) -> Result<HashSet<String>> {
    let mut all_feedback = HashSet::new();
    for url in ListenbrainzPaginator::new(
        &format!("https://api.listenbrainz.org/1/feedback/user/{user_name}/get-feedback"),
        0,
        1000,
    ) {
        let client = reqwest::Client::new();
        let real_url =
            Url::parse_with_params(url.as_ref(), [("score", (feedback as i8).to_string())])
                .expect("Could not construct url");
        let response = client.get(real_url).send().await?;
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
                .filter_map(|f| f.recording_mbid),
        )
    }
    Ok(all_feedback)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_get_existing_feedback() {
        let result = get_existing_feedback("Serene-Arc", Feedback::Love);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async { result.await }).unwrap();
        // Magic number for me specifically; I know I have more than 100 favourites
        assert!(result.len() > 100)
    }
}
