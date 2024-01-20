mod audio_data;
mod playlist;

use crate::playlist::{get_current_playlists, get_current_user};
use anyhow::{Error, Result};
use audio_data::AudioFileData;
use clap::{Parser, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::Config;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use governor::{Quota, RateLimiter};
use indicatif::ProgressBar;
use inquire::Confirm;
use log::{debug, error, info};
use m3u::Entry;
use num_traits::ToPrimitive;
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;


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
    #[arg(value_enum, short, long, default_value = "none")]
    duplicate_action: DuplicateAction,
    #[arg(short, long, default_value_t = false)]
    no_confirm: bool,
    #[arg(long, hide = true)]
    markdown_help: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lowercase")]
enum Feedback {
    LOVE = 1,
    HATE = -1,
    NEUTRAL = 0,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lowercase")]
enum DuplicateAction {
    NONE,
    OVERWRITE,
    NUMBER,
    ABORT,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    if args.markdown_help {
        clap_markdown::print_help_markdown::<Args>();
        exit(0)
    }

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

    
    let token = match settings.get_string("user_token") {
        Ok(t) => t,
        Err(_) => {
            error!("Configuration does not contain a token!");
            exit(1);
        }
    };

    debug!("Testing token by resolving to user");
    let user_name = match get_current_user(&token).await {
        Ok(s) => s,
        Err(e) => {
            error!("Could not resolve token successfully: {}", e);
            exit(1);
        }
    };
    info!("This token belongs to {}!", &user_name);

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
        .flat_map(audio_data::load_tags_from_file_path)
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

    info!("Resolving song tags to Musicbrainz IDs...");
    let musicbrainz_ids = resolve_all_songs_for_mbids(song_data).await;

    let number_of_resolved_songs = musicbrainz_ids.len();
    let percentage = calculate_percentage(number_of_resolved_songs, number_of_tagged_songs)
        .expect("Could not calculate percentage of resolved songs");
    info!(
        "{}/{} ({:.2}%) of songs were resolved",
        number_of_resolved_songs, number_of_tagged_songs, percentage,
    );

    match args.no_confirm {
        true => {}
        false => {
            match Confirm::new("Do you want to continue with the matched songs?")
                .with_default(true)
                .prompt()
            {
                Ok(true) => {
                    info!("Continuing");
                }
                Ok(false) => {
                    info!("Aborting");
                    exit(1)
                }
                Err(_) => {
                    error!("Error with questionaire");
                }
            }
        }
    }

    debug!("Retrieving existing playlists");
    let current_playlists = match get_current_playlists(&token, &user_name).await {
        Ok(playlists) => playlists,
        Err(e) => {
            error!("Could not retrieve existing playlists: {}", e);
            exit(1)
        }
    };
    info!(
        "Found {} existing playlists on account",
        current_playlists.len()
    );
    let mut playlist_name = args.playlist_name.clone();
    match current_playlists
        .iter()
        .any(|p| p.title == args.playlist_name)
    {
        true => {
            info!("Found a duplicate playlist, enacting duplicate policy");
            match args.duplicate_action {
                DuplicateAction::NONE => {}
                DuplicateAction::OVERWRITE => {
                    todo!()
                }
                DuplicateAction::NUMBER => {
                    for i in 1.. {
                        let prospective_title = format!("{}_{}", args.playlist_name, i);
                        if current_playlists
                            .iter()
                            .any(|p| p.title == prospective_title)
                        {
                            continue;
                        } else {
                            playlist_name = prospective_title;
                        }
                    }
                }
                DuplicateAction::ABORT => {
                    error!("Duplicate action says to abort!");
                    exit(1)
                }
            }
        }
        false => {}
    }

    debug!("Submitting new playlist");
    match playlist::submit_playlist(&token, &musicbrainz_ids, playlist_name, args.public).await {
        Ok(r) => {
            info!("Playlist created with ID {}", r.playlist_mbid);
        }
        Err(e) => {
            error!("Could not create playlist: {}", e);
        }
    }
    match args.feedback {
        None => {}
        Some(f) => {
            info!("Sending feedback for songs in playlist...");
            give_feedback_on_all_songs(&musicbrainz_ids, &token, f).await;
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
        .iter()
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
            Ok(()) => {}
            Err(e) => {
                error!("Could not give feedback on song: {}", e);
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
                let out = audio_data::get_musicbrainz_id_for_audio_data(data).await;
                pb.inc(1);
                out
            }
            .boxed()
        })
        .collect();

    let musicbrainz_ids: Vec<Result<String>> = futures.collect().await;
    
    musicbrainz_ids
        .into_iter()
        .filter_map(|result| match result {
            Ok(s) => Some(s),
            Err(e) => {
                error!("Could not resolve song: {}", e);
                None
            }
        })
        .collect()
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
        .header(AUTHORIZATION, format!("Token {user_token}"))
        .json(&parameters)
        .send()
        .await;
    match response {
        Ok(_) => Ok(()),
        Err(e) => Err(Error::from(e)),
    }
}

#[cfg(test)]
mod test {
    use crate::*;

    #[test]
    fn test_load_songs_from_playlist() {
        let file_path = &PathBuf::from("./tests/test_playlist_1.m3u");
        let result = load_file_paths(file_path);

        assert_eq!(result.len(), 4);
    }
}
