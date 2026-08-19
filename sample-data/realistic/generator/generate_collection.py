#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "mutagen>=1.47.0",
#   "numpy>=2.0.0",
#   "pyyaml>=6.0.0",
#   "soundfile>=0.13.0",
# ]
# ///
"""
Build the realistic sample collection described by `collection.yaml`.

Every track in the definition becomes a FLAC file carrying that track's real
metadata and exactly as much audio as the real recording runs for — but the
audio itself is digital silence, so no copyrighted recording is reproduced and
a full-length album costs the repository a few dozen kilobytes. FLAC stores a
run of identical samples as a constant subframe, which is why silence collapses
to roughly a tenth of a kilobyte per second.

Only what belongs in a file's tags is written here. The ratings and play logs
that `collection.yaml` also carries are the user's own data rather than the
file's, and are loaded into the database by other means.
"""

import shutil
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
import yaml
from mutagen.flac import FLAC

from layout import relative_path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFINITION = SCRIPT_DIR / "collection.yaml"
COLLECTION_DIR = SCRIPT_DIR.parent / "collection"

# Silence is written a chunk at a time so that an eight-minute track never has
# to exist in memory as a whole array of samples.
CHUNK_SECONDS = 30


def parse_duration(text: str) -> int:
    """Convert a `mm:ss` duration into a whole number of seconds."""
    minutes, seconds = text.split(":")
    return int(minutes) * 60 + int(seconds)


def track_path(template: str, album: dict, track: dict) -> Path:
    """Resolve a track's location under the collection root."""
    return COLLECTION_DIR / relative_path(template, album, track)


def write_silence(path: Path, seconds: int, audio: dict) -> None:
    """Write a FLAC file of `seconds` of silence, per the `audio` settings."""
    subtype = f"PCM_{audio['bit_depth']}"
    sample_rate = audio["sample_rate"]
    channels = audio["channels"]
    chunk = np.zeros((CHUNK_SECONDS * sample_rate, channels), dtype=np.int16)

    path.parent.mkdir(parents=True, exist_ok=True)
    with sf.SoundFile(
        path,
        mode="w",
        samplerate=sample_rate,
        channels=channels,
        subtype=subtype,
        format="FLAC",
    ) as flac:
        remaining = seconds * sample_rate
        while remaining > 0:
            flac.write(chunk[: min(remaining, len(chunk))])
            remaining -= len(chunk)


def write_tags(path: Path, album: dict, track: dict, track_total: int) -> None:
    """Tag a generated file with the track's real metadata.

    Tags that hold several values — ARTIST, GENRE — get one Vorbis comment per
    value rather than one delimited string, which is how the scanner expects to
    find a track credited to more than one artist. A track falls back to its
    album's artists and genres unless it names its own.
    """
    tags = FLAC(path)
    tags["title"] = track["title"]
    tags["artist"] = track.get("artists", album["artists"])
    tags["genre"] = track.get("genres", album["genres"])
    tags["album"] = album["title"]
    tags["albumartist"] = album["album_artist"]
    tags["date"] = str(album["year"])
    tags["tracknumber"] = str(track["number"])
    tags["tracktotal"] = str(track_total)
    tags.save()


def generate_collection() -> None:
    if not DEFINITION.exists():
        print(f"Error: no collection definition at {DEFINITION}", file=sys.stderr)
        sys.exit(1)

    definition = yaml.safe_load(DEFINITION.read_text())
    audio = definition["audio"]
    if audio["codec"] != "flac":
        print(f"Error: unsupported codec {audio['codec']!r}", file=sys.stderr)
        sys.exit(1)
    template = definition["path_template"]

    # The collection is rebuilt from scratch, so that a track renamed or dropped
    # in the definition doesn't leave a stale file behind to be scanned.
    if COLLECTION_DIR.exists():
        print(f"Removing existing collection at {COLLECTION_DIR}")
        shutil.rmtree(COLLECTION_DIR)

    print(f"Generating collection in {COLLECTION_DIR}")
    tracks_written = 0
    seconds_written = 0
    for album in definition["albums"]:
        print(f"\n{album['album_artist']} — {album['title']} ({album['year']})")
        track_total = len(album["tracks"])
        for track in album["tracks"]:
            path = track_path(template, album, track)
            seconds = parse_duration(track["duration"])
            write_silence(path, seconds, audio)
            write_tags(path, album, track, track_total)
            tracks_written += 1
            seconds_written += seconds
            print(f"  ✓ {path.relative_to(COLLECTION_DIR)}  ({track['duration']})")

    size = sum(f.stat().st_size for f in COLLECTION_DIR.rglob("*.flac"))
    print(
        f"\n✓ Wrote {tracks_written} tracks "
        f"({seconds_written // 60}:{seconds_written % 60:02d} of silence, "
        f"{size / 1024 / 1024:.1f} MiB) to {COLLECTION_DIR}"
    )


if __name__ == "__main__":
    generate_collection()
