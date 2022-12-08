use clap::*;
use id3::frame::Picture;
use id3::{Frame, Tag, TagLike};
use json::JsonValue;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    /// Output the tags and album art if present from the given audio file
    Extract,
    /// Given a JSON file containing tags, apply the tags to the given audio file
    Apply,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[arg(value_enum)]
    mode: Mode,
    /// The path to the file containing ID3 tags. This file must already exist, regardless of mode
    #[arg(value_parser = file_exists)]
    id3: PathBuf,
    /// The path to a JSON file that contails, or will contain, tag data. When extracting, this file will be recreated even if it already exists
    json: PathBuf,
    /// The path of the album art. When extracting, this is derived from the audio path if not given.
    art: Option<PathBuf>,
}

fn file_exists(path_str: &str) -> Result<PathBuf, String> {
    let path: PathBuf = path_str.into();
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("id3 file {path_str} not found"))
    }
}

/// Write the ID3 tags from the given file out as JSON. Also extract the album art to the given path if available
fn extract_tags(
    id3_file: PathBuf,
    json_path: PathBuf,
    album_path: Option<PathBuf>,
) -> Result<(), String> {
    let album_path = album_path.unwrap_or_else(|| id3_file.with_extension(".jpg"));

    let tag = match Tag::read_from_path(id3_file) {
        Ok(t) => t,
        Err(e) => Err(format!("Unable to open id3 file: {e}"))?, // No need to include the path because we know its valid already
    };

    let mut json = JsonValue::new_object();

    // Get all the text tags
    for frame in tag.frames() {
        if let Some(text) = frame.content().text() {
            json[frame.id()] = JsonValue::String(text.to_owned());
        }
    }

    let pretty_json = json::stringify_pretty(json, 4);

    let mut json_file = match File::create(&json_path) {
        Ok(file) => file,
        Err(e) => Err(format!("Cannot open {}: {e}", json_path.to_string_lossy()))?,
    };

    if let Err(e) = json_file.write_all(pretty_json.as_bytes()) {
        return Err(format!("Cannot write JSON: {e}",));
    };

    // The spec allows for multiple pictures but in practice there's only one
    if let Some(pic) = tag.pictures().next() {
        let mut file = match File::create(&album_path) {
            Ok(file) => file,
            Err(e) => Err(format!(
                "Cannot extract album art to {}: {e}",
                album_path.to_string_lossy()
            ))?,
        };

        if let Err(e) = file.write_all(&pic.data) {
            return Err(format!("Album art extraction failed: {e}"));
        };
    }

    Ok(())
}

fn apply_tags(
    id3_file: PathBuf,
    json_path: PathBuf,
    album_path: Option<PathBuf>,
) -> Result<(), String> {
    let json = match std::fs::read_to_string(json_path) {
        Ok(s) => s,
        Err(e) => Err(format!("Unable to open json file: {e}"))?,
    };
    let json = match json::parse(&json) {
        Ok(j) => j,
        Err(e) => Err(format!("Unable to parse JSON: {e}"))?,
    };

    if !json.is_object() {
        return Err(format!("No root object found"));
    }

    let mut tag = Tag::new();

    for (key, val) in json.entries() {
        if val.is_string() {
            let frame = Frame::text(key, val.to_string());
            tag.add_frame(frame);
        }
    }

    if let Some(album_path) = album_path {
        if album_path.exists() {
            let data = match std::fs::read(&album_path) {
                Ok(data) => data,
                Err(e) => Err(format!("Cannot read album art data: {e}"))?,
            };
            let picture = Picture {
                data,
                description: "".to_owned(),
                picture_type: id3::frame::PictureType::CoverFront,
                mime_type: "image/jpeg".to_owned(),
            };
            tag.add_frame(picture);
        } else {
            return Err(format!(
                "Provided album path does not exist: {}",
                album_path.to_string_lossy()
            ));
        }
    }

    if let Err(e) = tag.write_to_path(id3_file, id3::Version::Id3v24) {
        return Err(format!("Could not write tags: {e}"));
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.mode {
        Mode::Extract => extract_tags(cli.id3, cli.json, cli.art),
        Mode::Apply => apply_tags(cli.id3, cli.json, cli.art),
    };
    if let Err(s) = result {
        eprintln!("Something went wrong: {s}");
    }
}
