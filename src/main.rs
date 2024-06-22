mod audio_data;
mod feedback;
mod listenbrainz_client;
mod paginator;
mod playlist;

use crate::audio_data::AudioIDData;
use crate::feedback::get_existing_feedback;
use crate::listenbrainz_client::ListenbrainzClient;
use crate::playlist::{
    delete_items_from_playlist, get_current_playlists, get_current_user, mass_add_to_playlist,
    FullExistingPlaylistResponse,
};
use anyhow::Result;
use clap::{Parser, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::Config;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
use log::{debug, error, info};
use m3u::Entry;
use num_traits::ToPrimitive;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use tokio::sync::Mutex;

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
    Love = 1,
    Hate = -1,
    Neutral = 0,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
#[clap(rename_all = "lowercase")]
enum DuplicateAction {
    None,
    Overwrite,
    Number,
    Abort,
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

    let Ok(token) = settings.get_string("user_token") else {
        error!("Configuration does not contain a token!");
        exit(1)
    };

    let mut client = ListenbrainzClient::new(token);

    debug!("Testing token by resolving to user");
    let user_name = match get_current_user(&mut client).await {
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
    let percentage = calculate_percentage(&number_of_tagged_songs, &number_of_files)
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
    let musicbrainz_ids = resolve_all_songs_for_mbids(&mut client, song_data).await;

    let number_of_resolved_songs = musicbrainz_ids.len();
    let percentage = calculate_percentage(&number_of_resolved_songs, &number_of_tagged_songs)
        .expect("Could not calculate percentage of resolved songs");
    info!(
        "{}/{} ({:.2}%) of songs were resolved",
        number_of_resolved_songs, number_of_tagged_songs, percentage,
    );

    if !args.no_confirm {
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

    debug!("Retrieving existing playlists");
    let current_playlists = match get_current_playlists(&mut client, &user_name).await {
        Ok(playlists) => playlists,
        Err(e) => {
            error!("Could not retrieve existing playlists: {}", e);
            exit(1)
        }
    };
    debug!(
        "Found {} existing playlists on account",
        current_playlists.len()
    );
    let mut playlist_name = args.playlist_name.clone();
    let searched_playlist = current_playlists
        .iter()
        .find(|p| p.title == args.playlist_name);
    match searched_playlist {
        Some(p) => {
            let p = match FullExistingPlaylistResponse::convert_simple_playlist_response_to_full(
                &mut client,
                p,
            )
            .await
            {
                Err(e) => {
                    error!(
                        "Could not find more detailed information on possible duplicate playlist: {}",
                        e
                    );
                    exit(1)
                }
                Ok(p) => p,
            };
            info!("Found a duplicate playlist, enacting duplicate policy");
            match args.duplicate_action {
                DuplicateAction::None => {
                    // Just submit new playlist
                    submit_new_playlist(&mut client, args.public, &musicbrainz_ids, playlist_name)
                        .await;
                }
                DuplicateAction::Overwrite => {
                    if p.number_of_tracks > 0 {
                        let deletion_request = delete_items_from_playlist(
                            &mut client,
                            &p.identifier,
                            0,
                            p.number_of_tracks + 1,
                        )
                        .await;
                        match deletion_request {
                            Ok(()) => {}
                            Err(e) => {
                                error!(
                                    "Could not delete items from playlist to overwrite it: {}",
                                    e
                                );
                                exit(1)
                            }
                        }
                    } else {
                        debug!("Existing playlist already has no tracks");
                    }
                    let insertion_request =
                        mass_add_to_playlist(&mut client, &p.identifier, &musicbrainz_ids).await;
                    match insertion_request {
                        Ok(()) => {
                            info!("Replaced songs in playlist with ID {}", p.identifier);
                        }
                        Err(e) => {
                            error!("Could not insert new items into playlist: {}", e);
                            exit(1)
                        }
                    }
                }
                DuplicateAction::Number => {
                    for i in 1.. {
                        let prospective_title = format!("{}_{}", args.playlist_name, i);
                        if current_playlists
                            .iter()
                            .any(|p| p.title == prospective_title)
                        {
                            continue;
                        }
                        playlist_name = prospective_title;
                    }
                    submit_new_playlist(&mut client, args.public, &musicbrainz_ids, playlist_name)
                        .await;
                }
                DuplicateAction::Abort => {
                    error!("Duplicate action says to abort!");
                    exit(1)
                }
            }
        }
        None => {
            info!("No duplicate playlists found");
            submit_new_playlist(&mut client, args.public, &musicbrainz_ids, playlist_name).await;
        }
    }

    match args.feedback {
        None => {}
        Some(f) => {
            let given_feedback = get_existing_feedback(&mut client, &user_name, f)
                .await
                .expect("Could not get existing feedback");
            let filtered_musicbrainz_ids: Vec<_> = musicbrainz_ids
                .iter()
                .filter(|i| !given_feedback.contains(*i))
                .collect();
            let filtered_len = filtered_musicbrainz_ids.len();
            let total_len = musicbrainz_ids.len();
            let correct_len = total_len - filtered_len;
            let percentage = calculate_percentage(&correct_len, &total_len).unwrap();
            if filtered_len == 0 {
                info!("All songs in playlist already have the correct feedback");
                exit(0)
            } else if filtered_len == total_len {
                info!("Sending feedback for songs in playlist...");
            } else {
                info!(
                    "{}/{} ({:.2}%) of songs already have the correct feedback",
                    correct_len, total_len, percentage
                );
                info!("Sending feedback for remaining songs in playlist...");
            }
            give_feedback_on_all_songs(&mut client, filtered_musicbrainz_ids, f).await;
        }
    }
}

async fn submit_new_playlist(
    listenbrainz_client: &mut ListenbrainzClient,
    public: bool,
    musicbrainz_ids: &Vec<String>,
    playlist_name: String,
) {
    debug!("Submitting new playlist");
    match playlist::submit_playlist(listenbrainz_client, musicbrainz_ids, playlist_name, public)
        .await
    {
        Ok(r) => {
            info!("Playlist created with ID {}", r.playlist_mbid);
        }
        Err(e) => {
            error!("Could not create playlist: {}", e);
        }
    }
}

async fn give_feedback_on_all_songs(
    listenbrainz_client: &mut ListenbrainzClient,
    musicbrainz_ids: Vec<&String>,
    feedback: Feedback,
) {
    let progress_bar = make_progress_bar(musicbrainz_ids.len());
    let listenbrainz_client = Arc::new(Mutex::new(listenbrainz_client));
    let futures: FuturesUnordered<_> = musicbrainz_ids
        .iter()
        .map(|mbid| {
            let pb = Arc::clone(&progress_bar);
            let listenbrainz_client = Arc::clone(&listenbrainz_client);
            async move {
                let out = feedback::give_song_feedback_for_mbid(
                    *listenbrainz_client.lock().await,
                    mbid,
                    feedback,
                )
                .await;
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
async fn resolve_all_songs_for_mbids(
    listenbrainz_client: &mut ListenbrainzClient,
    song_data: Vec<AudioIDData>,
) -> Vec<String> {
    let progress_bar = make_progress_bar(song_data.len());
    let listenbrainz_client = Arc::new(Mutex::new(listenbrainz_client));
    let futures: FuturesUnordered<_> = song_data
        .into_iter()
        .map(|data| {
            let pb = Arc::clone(&progress_bar);
            let listenbrainz_client = Arc::clone(&listenbrainz_client);
            async move {
                let out = match data {
                    AudioIDData::Mbid(mbid) => Ok(mbid),
                    AudioIDData::AudioFileData(d) => {
                        audio_data::get_musicbrainz_id_for_audio_data(
                            *listenbrainz_client.lock().await,
                            d,
                        )
                        .await
                    }
                };
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

fn make_progress_bar(length: usize) -> Arc<ProgressBar> {
    Arc::new(ProgressBar::new(length as u64).with_style(
        ProgressStyle::with_template("[{elapsed_precise}] {wide_bar} {human_pos}/{human_len} ({percent}%) [{eta_precise}]").unwrap())
    )
}

fn calculate_percentage<T>(numerator: &T, denominator: &T) -> Option<f64>
where
    T: ToPrimitive,
{
    match (numerator.to_f64(), denominator.to_f64()) {
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
