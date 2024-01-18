mod playlist;

use anyhow::{anyhow, Error, Result};
use audiotags::Tag;
use clap::{Parser, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::Config;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use governor::{Quota, RateLimiter};
use indicatif::ProgressBar;
use log::{error, info};
use m3u::Entry;
use num_traits::ToPrimitive;
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
use tokio;
use url::Url;

#[derive(Parser, Debug)]
struct Args {
    file: PathBuf,
    #[arg(short, long, default_value = "./config.toml")]
    config: PathBuf,
    playlist_name: String,
    #[arg(value_enum, short, long)]
    feedback: Option<Feedback>,
    #[arg(short, long, default_value_t = false)]
    public: bool,
    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lowercase")]
enum Feedback {
    LOVE = 1,
    HATE = -1,
    NEUTRAL = 0,
}

#[derive(Debug, Eq, PartialEq)]
struct AudioFileData {
    artist: String,
    title: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let verbosity = args.verbose;
    env_logger::Builder::new()
        .filter_level(verbosity.log_level_filter())
        .init();

    let settings = Config::builder()
        .add_source(config::File::from(args.config))
        .build()
        .expect("Could not read configuration");

    if !args.file.exists() {
        error!("Given playlist file doesn't exist");
        exit(1);
    }

    let file_path = &args.file;
    let playlist_entries = load_file_paths(file_path);
    let number_of_files = playlist_entries.len();
    info!("Found {} files in playlist", number_of_files);

    if number_of_files == 0 {
        error!("No files read from playlist, aborting");
        exit(1);
    }

    let song_data: Vec<_> = playlist_entries
        .into_iter()
        .flat_map(|e| load_tags_from_file_path(e))
        .collect();
    let number_of_tagged_songs = song_data.len();
    let percentage = calculate_percentage(number_of_tagged_songs, number_of_files)
        .expect("Could not calculate percentage of tagged songs");
    info!(
        "{}/{} ({:.2}%) of songs had readable tags",
        number_of_tagged_songs, number_of_files, percentage,
    );

    if number_of_tagged_songs == 0 {
        error!("No tagged songs could be read, aborting");
        exit(1);
    }

    let token;
    match settings.get_string("user_token") {
        Ok(t) => token = t,
        Err(_) => {
            error!("Configuration does not contain a token!");
            exit(1);
        }
    }

    info!("Resolving song tags to Musicbrainz IDs...");
    let musicbrainz_ids = resolve_all_songs_for_mbids(song_data).await;

    let number_of_resolved_songs = musicbrainz_ids.len();
    let percentage = calculate_percentage(number_of_resolved_songs, number_of_tagged_songs)
        .expect("Could not calculate percentage of resolved songs");
    info!(
        "{}/{} ({:.2}%) of songs were resolved",
        number_of_resolved_songs, number_of_tagged_songs, percentage,
    );

    match playlist::submit_playlist(&token, &musicbrainz_ids, args.playlist_name, args.public).await
    {
        Ok(r) => {
            info!("Playlist created with ID {}", r.playlist_mbid)
        }
        Err(e) => {
            error!("Could not create playlist: {}", e)
        }
    }
    match args.feedback {
        None => {}
        Some(f) => {
            info!("Sending feedback for songs in playlist...");
            give_feedback_on_all_songs(&musicbrainz_ids, &token, f).await
        }
    }
}

async fn give_feedback_on_all_songs(
    musicbrainz_ids: &Vec<String>,
    user_token: &String,
    feedback: Feedback,
) {
    // Be a good internet citizen; this isn't an important application.
    let rate_limiter = Arc::new(RateLimiter::direct(
        Quota::with_period(Duration::from_secs(5)).expect("Could not create quota"),
    ));

    let progress_bar = Arc::new(ProgressBar::new(musicbrainz_ids.len() as u64));
    let futures: FuturesUnordered<_> = musicbrainz_ids
        .into_iter()
        .map(|mbid| {
            let limiter = Arc::clone(&rate_limiter);
            let pb = Arc::clone(&progress_bar);
            async move {
                limiter.until_ready().await;
                let out = give_song_feedback_for_mbid(user_token, mbid, feedback).await;
                pb.inc(1);
                out
            }
            .boxed()
        })
        .collect();

    let results: Vec<Result<()>> = futures.collect().await;
    for result in results {
        match result {
            Ok(_) => {}
            Err(e) => {
                error!("Could not give feedback on song: {}", e)
            }
        }
    }
}
async fn resolve_all_songs_for_mbids(song_data: Vec<AudioFileData>) -> Vec<String> {
    // Be a good internet citizen; this isn't an important application.
    let rate_limiter = Arc::new(RateLimiter::direct(
        Quota::with_period(Duration::from_secs(5)).expect("Could not create quota"),
    ));

    let progress_bar = Arc::new(ProgressBar::new(song_data.len() as u64));
    let futures: FuturesUnordered<_> = song_data
        .into_iter()
        .map(|data| {
            let limiter = Arc::clone(&rate_limiter);
            let pb = Arc::clone(&progress_bar);
            async move {
                limiter.until_ready().await;
                let out = get_musicbrainz_id_for_audio_data(data).await;
                pb.inc(1);
                out
            }
            .boxed()
        })
        .collect();

    let musicbrainz_ids: Vec<Result<String>> = futures.collect().await;
    let musicbrainz_ids = musicbrainz_ids
        .into_iter()
        .filter_map(|result| match result {
            Ok(s) => Some(s),
            Err(e) => {
                error!("Could not resolve song: {}", e);
                None
            }
        })
        .collect();
    musicbrainz_ids
}

fn calculate_percentage<T>(first: T, second: T) -> Option<f64>
where
    T: ToPrimitive,
{
    match (first.to_f64(), second.to_f64()) {
        (Some(first), Some(second)) if second != 0.0 => Some((first / second) * 100.0),
        _ => None,
    }
}

fn load_tags_from_file_path(playlist_entries: PathBuf) -> Result<AudioFileData> {
    let tags = Tag::new().read_from_path(playlist_entries)?;
    let artist = tags
        .artist()
        .ok_or(anyhow!("Could not read artist"))?
        .parse()?;
    let title = tags
        .title()
        .ok_or(anyhow!("Could not read title"))?
        .parse()?;
    Ok(AudioFileData { artist, title })
}

fn load_file_paths(file_path: &PathBuf) -> Vec<PathBuf> {
    let playlist_entries: Vec<PathBuf> = m3u::Reader::open(file_path)
        .expect("Could not read playlist file")
        .entries()
        .map(|e| e.expect("Could not read M3U entry"))
        .filter_map(|e| match e {
            Entry::Path(path) => Some(path),
            Entry::Url(_) => None,
        })
        .collect();
    playlist_entries
}

async fn give_song_feedback_for_mbid(
    user_token: &String,
    mbid: &String,
    feedback: Feedback,
) -> Result<()> {
    let client = reqwest::Client::new();
    let parameters = json!({"recording_mbid": mbid,"score": &(feedback as i8).to_string(),});
    let response = client
        .post("https://api.listenbrainz.org/1/feedback/recording-feedback")
        .header(AUTHORIZATION, format!("Token {}", user_token))
        .json(&parameters)
        .send()
        .await;
    return match response {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::from(e)),
    };
}

async fn get_musicbrainz_id_for_audio_data(audio_file_data: AudioFileData) -> Result<String> {
    let request_url: Url = Url::parse_with_params(
        "https://api.listenbrainz.org/1/metadata/lookup/",
        &[
            ("artist_name", audio_file_data.artist.clone()),
            ("recording_name", audio_file_data.title.clone()),
        ],
    )?;
    let result = reqwest::get(request_url)
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;

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
    use crate::*;

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

    #[test]
    fn test_load_songs_from_playlist() {
        let file_path = &PathBuf::from("./tests/test_playlist_1.m3u");
        let result = load_file_paths(file_path);

        assert_eq!(result.len(), 4)
    }
}
