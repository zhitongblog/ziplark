use crate::error::{Error, Result};
use crate::formats::ensure_parent;
use crate::model::*;
use crate::{CreateOptions, ExtractOptions, Level, ListOptions, ProgressFn};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The decompressed name for a single-stream archive: drop the trailing
/// compression extension. `notes.txt.gz` -> `notes.txt`, `blob.zst` -> `blob`.
fn inner_name(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("output");
    for ext in [".gz", ".bz2", ".xz", ".zst", ".lz4", ".tgz", ".tbz2", ".txz", ".tzst", ".tlz4"] {
        if let Some(stripped) = name.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    format!("{name}.out")
}

fn decoder(path: &Path, fmt: Format) -> Result<Box<dyn Read>> {
    let f = File::open(path)?;
    Ok(match fmt {
        Format::Gz => Box::new(flate2::read::GzDecoder::new(f)),
        Format::Bz2 => Box::new(bzip2::read::BzDecoder::new(f)),
        Format::Xz => Box::new(xz2::read::XzDecoder::new(f)),
        Format::Zst => Box::new(zstd::stream::read::Decoder::new(f)?),
        Format::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(f)),
        _ => return Err(Error::UnsupportedFormat(Some(path.to_path_buf()))),
    })
}

pub fn list(path: &Path, fmt: Format, _opts: &ListOptions) -> Result<ArchiveInfo> {
    let compressed = std::fs::metadata(path)?.len();
    // A single stream stores no uncompressed size; report it as unknown (0).
    let entry = ArchiveEntry {
        path: inner_name(path),
        is_dir: false,
        size: 0,
        compressed_size: Some(compressed),
        encrypted: false,
        modified: None,
        crc32: None,
    };
    Ok(ArchiveInfo {
        format: fmt,
        path: path.to_path_buf(),
        entries: vec![entry],
        encrypted: false,
        total_size: 0,
        total_compressed: compressed,
    })
}

pub fn extract(
    path: &Path,
    fmt: Format,
    opts: &ExtractOptions,
    progress: ProgressFn,
) -> Result<ExtractReport> {
    std::fs::create_dir_all(&opts.dest)?;
    let name = inner_name(path);
    let out_path = opts.dest.join(&name);
    ensure_parent(&out_path)?;
    if out_path.exists() && !opts.overwrite {
        return Err(Error::other(format!(
            "{} already exists (use overwrite)",
            out_path.display()
        )));
    }
    let mut dec = decoder(path, fmt)?;
    let mut out = File::create(&out_path)?;
    let n = io::copy(&mut dec, &mut out).map_err(map_stream_err)?;
    progress(Progress {
        current_path: name,
        entries_done: 1,
        entries_total: 1,
        bytes_done: n,
        bytes_total: n,
    });
    Ok(ExtractReport {
        files_written: 1,
        dirs_created: 0,
        bytes_written: n,
        dest: opts.dest.clone(),
    })
}

pub fn test(path: &Path, fmt: Format, _opts: &ListOptions, progress: ProgressFn) -> Result<TestReport> {
    let mut dec = decoder(path, fmt)?;
    let mut sink = io::sink();
    let mut bad = Vec::new();
    if let Err(e) = io::copy(&mut dec, &mut sink) {
        bad.push(format!("{}: {e}", path.display()));
    }
    progress(Progress {
        current_path: inner_name(path),
        entries_done: 1,
        entries_total: 1,
        bytes_done: 0,
        bytes_total: 0,
    });
    Ok(TestReport {
        ok: bad.is_empty(),
        entries_tested: 1,
        bad_entries: bad,
    })
}

pub fn create(
    output: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    progress: ProgressFn,
) -> Result<CreateReport> {
    if inputs.len() != 1 || inputs[0].is_dir() {
        return Err(Error::other(format!(
            "{} can only compress a single file; use a .tar.{} or .zip for multiple files/directories",
            opts.format.label(),
            opts.format.extension()
        )));
    }
    let src = &inputs[0];
    let mut input = File::open(src)?;
    let bytes_in = input.metadata()?.len();
    let out = File::create(output)?;

    match opts.format {
        Format::Gz => {
            let mut enc = flate2::write::GzEncoder::new(out, gz_level(opts.level));
            io::copy(&mut input, &mut enc)?;
            enc.finish()?;
        }
        Format::Bz2 => {
            let mut enc = bzip2::write::BzEncoder::new(out, bz_level(opts.level));
            io::copy(&mut input, &mut enc)?;
            enc.finish()?;
        }
        Format::Xz => {
            let mut enc = xz2::write::XzEncoder::new(out, xz_level(opts.level));
            io::copy(&mut input, &mut enc)?;
            enc.finish()?;
        }
        Format::Zst => {
            let mut enc = zstd::stream::write::Encoder::new(out, zst_level(opts.level))?;
            io::copy(&mut input, &mut enc)?;
            enc.finish()?;
        }
        Format::Lz4 => {
            // LZ4 frame format has no real "level" knob; block size is fixed default.
            let mut enc = lz4_flex::frame::FrameEncoder::new(out);
            io::copy(&mut input, &mut enc)?;
            enc.finish().map_err(|e| Error::other(e.to_string()))?;
        }
        _ => return Err(Error::UnsupportedFormat(Some(output.to_path_buf()))),
    }

    let mut sink_out = File::open(output)?;
    let _ = sink_out.flush();
    let bytes_out = std::fs::metadata(output)?.len();
    progress(Progress {
        current_path: src.display().to_string(),
        entries_done: 1,
        entries_total: 1,
        bytes_done: bytes_in,
        bytes_total: bytes_in,
    });
    Ok(CreateReport {
        output: output.to_path_buf(),
        format: opts.format,
        entries_added: 1,
        bytes_in,
        bytes_out,
    })
}

fn map_stream_err(e: io::Error) -> Error {
    Error::corrupt(e.to_string())
}

fn gz_level(l: Level) -> flate2::Compression {
    match l {
        Level::Store => flate2::Compression::none(),
        Level::Fast => flate2::Compression::fast(),
        Level::Default => flate2::Compression::default(),
        Level::Best => flate2::Compression::best(),
    }
}
fn bz_level(l: Level) -> bzip2::Compression {
    match l {
        Level::Store | Level::Fast => bzip2::Compression::fast(),
        Level::Default => bzip2::Compression::default(),
        Level::Best => bzip2::Compression::best(),
    }
}
fn xz_level(l: Level) -> u32 {
    match l {
        Level::Store => 0,
        Level::Fast => 1,
        Level::Default => 6,
        Level::Best => 9,
    }
}
fn zst_level(l: Level) -> i32 {
    match l {
        Level::Store => 1,
        Level::Fast => 3,
        Level::Default => 9,
        Level::Best => 19,
    }
}
