use clap::*;
use id3::frame::Picture;
use id3::{Frame, Tag, TagLike};
use json::JsonValue;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

type StrResult<T> = Result<T, String>;

#[derive(Args, Clone)]
struct BatchOpts {
    /// The files to extract
    files: Vec<PathBuf>,
    /// Aggregates output into a single JSON blob on stdout rather than individual files
    #[arg(short, long, default_value_t = false)]
    aggregate_output: bool,
    /// Recurses into any found directories
    #[arg(short, long, default_value_t = true)]
    recurse: bool,
}

#[derive(Args, Clone)]
struct SingleOpts {
    #[arg(value_parser = file_exists)]
    id3: PathBuf,
    /// The path to a JSON file that contails, or will contain, tag data. When extracting, this file will be recreated even if it already exists
    json: Option<PathBuf>,
    /// The path of the album art.
    art: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Mode {
    /// Output the tags and album art if present from the given audio file. Missing paths are derived from the id3 filename and existing files overwritten
    Extract(SingleOpts),
    /// Given a JSON file containing tags, apply the tags to the given audio file
    Apply(SingleOpts),
    /// Given a list of filenames, extract the tags and albums to correspondingly named files
    BatchExtract(BatchOpts),
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

fn file_exists(path_str: &str) -> Result<PathBuf, String> {
    let path: PathBuf = path_str.into();
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("id3 file {path_str} not found"))
    }
}

fn write_data_to_path(path: &PathBuf, data: &[u8]) -> StrResult<()> {
    let mut file = match File::create(path) {
        Ok(file) => file,
        Err(e) => Err(format!("Cannot open {}: {e}", path.to_string_lossy()))?,
    };
    if let Err(e) = file.write_all(data) {
        return Err(format!("Cannot write JSON: {e}",));
    };
    Ok(())
}

fn extract_tags_pic(id3_file: &PathBuf) -> StrResult<(JsonValue, Option<Vec<u8>>)> {
    let tag = match Tag::read_from_path(id3_file) {
        Ok(t) => t,
        Err(e) => Err(format!("Unable to open id3 file: {e}"))?, // No need to include the path because we know its valid already
    };
    let mut json = JsonValue::new_object();
    for frame in tag.frames() {
        if let Some(text) = frame.content().text() {
            json[frame.id()] = JsonValue::String(text.to_owned());
        }
    }
    let data = tag.pictures().next().map(|p| p.data.clone());
    Ok((json, data))
}

/// Write the ID3 tags from the given file out as JSON. Also extract the album art to the given path if available
fn extract_file(opts: SingleOpts) -> StrResult<()> {
    let art_path = opts.art.unwrap_or_else(|| opts.id3.with_extension(".jpg"));
    let json_path = opts
        .json
        .unwrap_or_else(|| opts.id3.with_extension(".json"));

    let (json, data) = extract_tags_pic(&opts.id3)?;
    let pretty_json = json::stringify_pretty(json, 4);

    write_data_to_path(&json_path, pretty_json.as_bytes())?;

    if let Some(data) = data {
        write_data_to_path(&art_path, &data)?;
    }

    Ok(())
}

fn apply_tags(opts: SingleOpts) -> StrResult<()> {
    let json_path = opts
        .json
        .unwrap_or_else(|| opts.id3.with_extension(".json"));
    let json = match std::fs::read_to_string(json_path) {
        Ok(s) => s,
        Err(e) => Err(format!("Unable to open json file: {e}"))?,
    };
    let json = match json::parse(&json) {
        Ok(j) => j,
        Err(e) => Err(format!("Unable to parse JSON: {e}"))?,
    };

    if !json.is_object() {
        return Err("No root object found".to_string());
    }

    let mut tag = Tag::new();

    for (key, val) in json.entries() {
        if val.is_string() {
            let frame = Frame::text(key, val.to_string());
            tag.add_frame(frame);
        }
    }

    if let Some(album_path) = opts.art {
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

    if let Err(e) = tag.write_to_path(opts.id3, id3::Version::Id3v24) {
        return Err(format!("Could not write tags: {e}"));
    }
    Ok(())
}

fn batch_extract(blob: &mut JsonValue, opt: &BatchOpts) -> StrResult<()> {
    for file in &opt.files {
        if file.is_dir() && opt.recurse {
            let contents = file.read_dir().unwrap();
            let files = contents.filter_map(Result::ok).map(|d| d.path()).collect();
            let opt = BatchOpts { files, ..*opt };
            batch_extract(blob, &opt)?;
        } else if file.is_file() {
            if !file.to_string_lossy().ends_with("mp3") {
                continue;
            }
            let (json, pic) = match extract_tags_pic(file) {
                Ok((j, p)) => (j, p),
                Err(s) => {
                    eprintln!("Could not handle {}: {}", file.to_string_lossy(), s);
                    continue;
                }
            };
            if let Some(pic) = pic {
                write_data_to_path(&file.with_extension("jpeg"), &pic)?;
            }
            if opt.aggregate_output {
                let path = file.to_string_lossy();
                blob[&*path] = json;
            } else {
                let json = json::stringify_pretty(json, 4);
                write_data_to_path(&file.with_extension("json"), json.as_bytes())?;
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.mode {
        Mode::Extract(opts) => extract_file(opts),
        Mode::Apply(opts) => apply_tags(opts),
        Mode::BatchExtract(opt) => {
            let mut blob = JsonValue::new_object();
            batch_extract(&mut blob, &opt)?;
            if opt.aggregate_output {
                let json = json::stringify_pretty(blob, 4);
                println!("{}", json);
            }
            Ok(())
        }
    }
}
