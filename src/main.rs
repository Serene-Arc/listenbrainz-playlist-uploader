use anyhow::{anyhow, Result};
use audiotags::Tag;
use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::Config;
use log::{error, info};
use m3u::Entry;
use serde_json::Value;
use std::hash::Hash;
use std::path::PathBuf;
use std::process::exit;
use url::Url;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    file: PathBuf,
    #[arg(short, long, default_value = "./config.toml")]
    config: PathBuf,
    #[arg(short, long)]
    playlist_name: Option<String>,
    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
}

struct AudioFileData {
    artist: String,
    title: String,
}

struct APIResponse {
    recording_mbid: String,
}

fn main() {
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

    let file_data = read_playlist(&args.file);

    let token;
    match settings.get_string("user_token") {
        Ok(t) => token = t,
        Err(_) => {
            error!("Configuration does not contain a token!");
            exit(1);
        }
    }
}

fn read_playlist(file_path: &PathBuf) -> Vec<AudioFileData> {
    let playlist_entries: Vec<PathBuf> = m3u::Reader::open(file_path)
        .expect("Could not read playlist file")
        .entries()
        .map(|e| e.expect("Could not read M3U entry"))
        .filter_map(|e| match e {
            Entry::Path(path) => Some(path),
            Entry::Url(_) => None,
        })
        .collect();
    info!("Found {} files in playlist", playlist_entries.len());

    let file_tags: Vec<_> = playlist_entries
        .iter()
        .flat_map(|e| Tag::new().read_from_path(e))
        .map(|tags| AudioFileData {
            artist: tags
                .artist()
                .expect("Artist could not be read")
                .parse()
                .unwrap(),
            title: tags.title().expect("Could not read title").parse().unwrap(),
        })
        .collect();
    file_tags
}

fn get_musicbrainz_id(audio_file_data: AudioFileData) -> Result<String> {
    let request_url: Url = Url::parse_with_params(
        "https://api.listenbrainz.org/1/metadata/lookup/",
        &[
            ("artist_name", audio_file_data.artist),
            ("recording_name", audio_file_data.title),
        ],
    )?;
    let result = reqwest::blocking::get(request_url)?
        .error_for_status()?
        .json::<Value>()?;

    let out = result
        .get("recording_mbid")
        .ok_or_else(|| anyhow::anyhow!("Could not extract recording MBID from JSON"))?
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
        let result = get_musicbrainz_id(test).unwrap();
        assert_eq!(result, "b84dd2d1-2bf1-4fcc-aadc-6cc39c36ba35")
    }

    #[test]
    #[should_panic]
    fn test_get_recording_mbid_fail_1() {
        let test = AudioFileData {
            artist: "Ed Sheeran".parse().unwrap(),
            title: "Asdjkhfgds".parse().unwrap(),
        };
        let result = get_musicbrainz_id(test).unwrap();
        assert_eq!(result, "b84dd2d1-2bf1-4fcc-aadc-6cc39c36ba35")
    }
}
