//! A downloaded subtitle as one file's bytes: ZIP and RAR archives give the
//! subtitle for the asked episode (or the largest one for a movie), gzip is
//! unpacked, anything else is the subtitle itself. Text formats are read
//! later by [`crate::formats`].
//!
//! Every archive is read with caps on the subtitle's size and on what is
//! decoded on the way to it, so a hostile archive cannot fill the memory.

use std::io::Read;

use crate::sources::{Query, SourceError, episode_marks};

/// Largest subtitle taken out of an archive.
pub const MAX_SUBTITLE: u64 = 2 * 1024 * 1024;
/// Most bytes a RAR archive may decode to reach one subtitle (a solid
/// archive decodes the members before it too).
const MAX_DECODED: u64 = 16 * 1024 * 1024;
/// Largest RAR 5 dictionary accepted; subtitle archives use far less.
const MAX_DICTIONARY: u64 = 64 * 1024 * 1024;
/// Most members an archive may list.
const MAX_MEMBERS: usize = 1000;

/// Subtitle files an archive may hold, preferred first: an SRT is taken
/// over the others when both are there.
const EXTENSIONS: [&str; 4] = [".srt", ".ass", ".ssa", ".vtt"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Zip,
    Rar,
    Gzip,
    /// 7-Zip: not read.
    SevenZip,
    Plain,
}

fn kind(body: &[u8]) -> Kind {
    if body.starts_with(b"PK\x03\x04") || body.starts_with(b"PK\x05\x06") {
        Kind::Zip
    } else if body.starts_with(b"Rar!\x1a\x07") || body.starts_with(b"RE~^") {
        Kind::Rar
    } else if body.starts_with(&[0x1f, 0x8b]) {
        Kind::Gzip
    } else if body.starts_with(b"7z\xbc\xaf\x27\x1c") {
        Kind::SevenZip
    } else {
        Kind::Plain
    }
}

/// The subtitle file in `body`, for `query`'s episode.
pub fn subtitle(body: Vec<u8>, query: &Query) -> Result<Vec<u8>, SourceError> {
    match kind(&body) {
        Kind::Zip => from_zip(&body, query),
        Kind::Rar => from_rar(&body, query),
        Kind::Gzip => {
            let inner = gunzip(&body)?;
            // A gzip around an archive is opened once more, not further.
            match kind(&inner) {
                Kind::Zip => from_zip(&inner, query),
                Kind::Rar => from_rar(&inner, query),
                Kind::Plain => Ok(inner),
                Kind::Gzip | Kind::SevenZip => Err(SourceError::Decode),
            }
        }
        Kind::SevenZip => Err(SourceError::Decode),
        Kind::Plain => Ok(body),
    }
}

fn gunzip(body: &[u8]) -> Result<Vec<u8>, SourceError> {
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(body)
        .take(MAX_SUBTITLE + 1)
        .read_to_end(&mut out)
        .map_err(|_| SourceError::Decode)?;
    if out.len() as u64 > MAX_SUBTITLE {
        return Err(SourceError::TooLarge);
    }
    Ok(out)
}

/// One subtitle file listed in an archive.
struct Member {
    index: usize,
    size: u64,
    /// Place of its extension in [`EXTENSIONS`].
    rank: usize,
    marks: Vec<(u32, u32)>,
}

fn member(index: usize, name: &[u8], size: u64) -> Option<Member> {
    // Old archives store names in a legacy code page; only the extension
    // and episode marks matter here, so a lossy read is enough.
    let name = String::from_utf8_lossy(name);
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("__macosx") || lower.contains("/__macosx/") {
        return None;
    }
    let rank = EXTENSIONS.iter().position(|ext| lower.ends_with(ext))?;
    Some(Member {
        index,
        size,
        rank,
        marks: episode_marks(&name),
    })
}

/// For an episode, the file named for it (or the only one); for a movie,
/// the largest. Only the most preferred format present is considered.
fn choose(members: &[Member], query: &Query) -> Result<usize, SourceError> {
    let best = members
        .iter()
        .map(|m| m.rank)
        .min()
        .ok_or(SourceError::NoSubtitle)?;
    let files: Vec<&Member> = members.iter().filter(|m| m.rank == best).collect();
    let chosen = match (query.season, query.episode) {
        (Some(season), Some(episode)) => {
            let named: Vec<_> = files
                .iter()
                .filter(|m| m.marks.contains(&(season, episode)))
                .collect();
            match named.as_slice() {
                [] if files.len() == 1 => files.first(),
                named => named.iter().copied().max_by_key(|m| m.size),
            }
        }
        _ => files.iter().max_by_key(|m| m.size),
    };
    let chosen = chosen.ok_or(SourceError::NoSubtitle)?;
    if chosen.size > MAX_SUBTITLE {
        return Err(SourceError::TooLarge);
    }
    Ok(chosen.index)
}

fn from_zip(body: &[u8], query: &Query) -> Result<Vec<u8>, SourceError> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(body)).map_err(|_| SourceError::Decode)?;
    if zip.len() > MAX_MEMBERS {
        return Err(SourceError::TooLarge);
    }
    let mut members = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        if file.is_file()
            && let Some(m) = member(i, file.name_raw(), file.size())
        {
            members.push(m);
        }
    }
    let index = choose(&members, query)?;
    let mut bytes = Vec::new();
    // The listed size is the archive's word; the read is capped anyway.
    zip.by_index(index)
        .map_err(|_| SourceError::Decode)?
        .take(MAX_SUBTITLE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SourceError::Decode)?;
    if bytes.len() as u64 > MAX_SUBTITLE {
        return Err(SourceError::TooLarge);
    }
    Ok(bytes)
}

fn from_rar(body: &[u8], query: &Query) -> Result<Vec<u8>, SourceError> {
    let options = rars::ArchiveReadOptions::new()
        .with_max_header_count(MAX_MEMBERS as u64 * 2 + 8)
        .with_max_header_bytes(1024 * 1024)
        .with_max_member_output_bytes(MAX_DECODED)
        .with_max_total_output_bytes(MAX_DECODED)
        .with_max_reader_workspace_bytes(MAX_DICTIONARY * 2)
        .with_rar50_dictionary_size_limit(MAX_DICTIONARY);
    let archive =
        rars::ArchiveReader::read_with_options(body, options).map_err(|_| SourceError::Decode)?;
    let mut members = Vec::new();
    for (i, m) in archive.members().enumerate().take(MAX_MEMBERS + 1) {
        if i == MAX_MEMBERS {
            return Err(SourceError::TooLarge);
        }
        let meta = &m.meta;
        // A password protected, linked or split member cannot be read alone.
        if meta.is_directory
            || meta.is_redirection
            || meta.is_encrypted
            || meta.is_split_before
            || meta.is_split_after
        {
            continue;
        }
        if let Some(m) = member(i, &meta.name, meta.unpacked_size) {
            members.push(m);
        }
    }
    let index = choose(&members, query)?;
    let bytes = archive
        .read_member_at_with_options(index, options)
        .map_err(|_| SourceError::Decode)?
        .ok_or(SourceError::NoSubtitle)?;
    if bytes.len() as u64 > MAX_SUBTITLE {
        return Err(SourceError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn movie() -> Query {
        Query::new("movie", "tt0133093", "tur").unwrap()
    }

    fn episode() -> Query {
        Query::new("series", "tt0903747:1:2", "tur").unwrap()
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn rar_of(version: rars::ArchiveVersion, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = rars::Builder::new(version);
        for (name, data) in files {
            builder
                .add_bytes(name.as_bytes().to_vec(), data.to_vec(), None, None)
                .unwrap();
        }
        builder.to_bytes().unwrap()
    }

    fn gzip_of(data: &[u8]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(data).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn plain_files_pass_through() {
        let srt = b"1\n00:00:01,000 --> 00:00:02,000\nHi\n".to_vec();
        assert_eq!(subtitle(srt.clone(), &movie()).unwrap(), srt);
    }

    #[test]
    fn zip_gives_the_episode_or_the_largest() {
        let pack = zip_of(&[
            ("Show.S01E01.srt", b"one"),
            ("Show.S01E02.srt", b"two"),
            ("Show.S01E02.HI.srt", b"two, longer"),
            ("__MACOSX/Show.S01E02.srt", b"junk junk junk junk"),
            ("readme.txt", b"hello hello hello hello"),
        ]);
        assert_eq!(subtitle(pack, &episode()).unwrap(), b"two, longer");
        let single = zip_of(&[("tr.srt", b"only")]);
        assert_eq!(subtitle(single, &episode()).unwrap(), b"only");
        let other = zip_of(&[("Show.S01E01.srt", b"one"), ("Show.S01E03.srt", b"three")]);
        assert_eq!(subtitle(other, &episode()), Err(SourceError::NoSubtitle));
        let zip = zip_of(&[("a.srt", b"short"), ("b.srt", b"the longest")]);
        assert_eq!(subtitle(zip, &movie()).unwrap(), b"the longest");
        let none = zip_of(&[("notes.txt", b"x")]);
        assert_eq!(subtitle(none, &movie()), Err(SourceError::NoSubtitle));
    }

    #[test]
    fn srt_wins_over_other_formats_and_others_are_taken_alone() {
        let zip = zip_of(&[
            ("Movie.ass", b"a much longer ass file"),
            ("Movie.srt", b"srt"),
        ]);
        assert_eq!(subtitle(zip, &movie()).unwrap(), b"srt");
        for name in ["Movie.ass", "Movie.SSA", "Movie.vtt"] {
            let zip = zip_of(&[(name, b"text"), ("__MACOSX/._Movie.srt", b"junk")]);
            assert_eq!(subtitle(zip, &movie()).unwrap(), b"text", "{name}");
        }
        let zip = zip_of(&[("Movie.nfo", b"x"), ("Movie.sub", b"y")]);
        assert_eq!(subtitle(zip, &movie()), Err(SourceError::NoSubtitle));
    }

    #[test]
    fn rar_old_and_new_give_the_episode() {
        for version in [rars::ArchiveVersion::Rar29, rars::ArchiveVersion::Rar50] {
            let rar = rar_of(
                version,
                &[
                    ("Show.S01E01.srt", b"one"),
                    ("Show.S01E02.srt", b"two two"),
                    ("Show.S01E03.srt", b"three"),
                ],
            );
            assert_eq!(
                subtitle(rar.clone(), &episode()).unwrap(),
                b"two two",
                "{version:?}"
            );
            let rar = rar_of(version, &[("Show.S01E05.srt", b"five")]);
            assert_eq!(subtitle(rar, &episode()).unwrap(), b"five");
        }
    }

    #[test]
    fn rar_solid_member_is_decoded_after_the_ones_before() {
        let mut builder = rars::Builder::new(rars::ArchiveVersion::Rar50).solid(true);
        for (name, data) in [("a.txt", "x".repeat(5000)), ("Movie.srt", "y".repeat(100))] {
            builder
                .add_bytes(name.into(), data.into_bytes(), None, None)
                .unwrap();
        }
        let rar = builder.to_bytes().unwrap();
        assert_eq!(subtitle(rar, &movie()).unwrap(), "y".repeat(100).as_bytes());
    }

    #[test]
    fn gzip_is_unpacked_once() {
        let srt = b"1\n00:00:01,000 --> 00:00:02,000\nHi\n";
        assert_eq!(subtitle(gzip_of(srt), &movie()).unwrap(), srt);
        let zip = zip_of(&[("Movie.srt", srt)]);
        assert_eq!(subtitle(gzip_of(&zip), &movie()).unwrap(), srt);
        assert_eq!(
            subtitle(gzip_of(&gzip_of(srt)), &movie()),
            Err(SourceError::Decode)
        );
    }

    #[test]
    fn broken_unknown_and_huge_archives_are_refused() {
        assert_eq!(
            subtitle(b"PK\x03\x04garbage".to_vec(), &movie()),
            Err(SourceError::Decode)
        );
        assert_eq!(
            subtitle(b"Rar!\x1a\x07\x01\x00garbage".to_vec(), &movie()),
            Err(SourceError::Decode)
        );
        assert_eq!(
            subtitle(b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec(), &movie()),
            Err(SourceError::Decode)
        );
        // Compresses to almost nothing; the cap stops the read, not the size field.
        let big = vec![b'a'; MAX_SUBTITLE as usize + 10];
        let zip = zip_of(&[("Movie.srt", &big)]);
        assert_eq!(subtitle(zip, &movie()), Err(SourceError::TooLarge));
        let rar = rar_of(rars::ArchiveVersion::Rar50, &[("Movie.srt", &big)]);
        assert_eq!(subtitle(rar, &movie()), Err(SourceError::TooLarge));
        assert_eq!(
            subtitle(gzip_of(&big), &movie()),
            Err(SourceError::TooLarge)
        );
    }
}
