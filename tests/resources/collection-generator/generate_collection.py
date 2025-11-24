#!/usr/bin/env python3
"""
Generate a test audio collection using piper-tts.

This script generates FLAC audio files for testing the collectune application.
It uses piper-tts to synthesize speech and converts the output to FLAC format
with appropriate metadata.
"""

import os
import shutil
import subprocess
import sys
from pathlib import Path

try:
    from mutagen.flac import FLAC
except ImportError:
    print("Error: mutagen is not installed. Run: uv sync", file=sys.stderr)
    sys.exit(1)

try:
    from piper import Piper
except ImportError:
    print("Error: piper-tts is not installed. Run: uv sync", file=sys.stderr)
    sys.exit(1)


# Define the output directory relative to this script
SCRIPT_DIR = Path(__file__).parent
COLLECTION_DIR = SCRIPT_DIR.parent / "collection"
ALBUM_DIR = COLLECTION_DIR / "The Announcers - First Test"

# Track definitions
TRACKS = [
    {"title": "Hen", "text": "One hen"},
    {"title": "Ducks", "text": "Two ducks"},
    {"title": "Geese", "text": "Three squawking geese"},
    {"title": "Oysters", "text": "Four limerick oysters"},
    {"title": "Porpoises", "text": "Five corpulent porpoises"},
    {"title": "Tweezers", "text": "Six pairs of Don Alverzo's tweezers"},
    {"title": "Macedonians", "text": "Seven thousand Macedonians in full battle array"},
    {"title": "Monkeys", "text": "Eight brass monkeys from the ancient sacred crypts of Egypt"},
    {"title": "Men", "text": "Nine apathetic, sympathetic, diabetic old men on roller skates with a marked propensity toward procrastination and sloth"},
    {"title": "Denizens", "text": "Ten lyrical, spherical, diabolical denizens of the deep who haul, stall, crawl, and creep through the coral reefs of the Caribbean searching for the share of the sherry sunken ship"},
]


def check_ffmpeg():
    """Check if ffmpeg is available."""
    try:
        subprocess.run(
            ["ffmpeg", "-version"],
            capture_output=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        print("Error: ffmpeg is not installed or not in PATH", file=sys.stderr)
        print("Install it with: sudo apt-get install ffmpeg (or equivalent)", file=sys.stderr)
        sys.exit(1)


def generate_collection():
    """Generate the audio collection."""
    # Check dependencies
    check_ffmpeg()
    
    # Clean and create output directory
    if ALBUM_DIR.exists():
        print(f"Removing existing collection at {ALBUM_DIR}")
        shutil.rmtree(ALBUM_DIR)
    ALBUM_DIR.mkdir(parents=True, exist_ok=True)
    
    print(f"Generating collection in {ALBUM_DIR}")
    print(f"Using piper-tts with model: en_GB-alan-high")
    
    # Initialize Piper TTS
    try:
        piper = Piper(model="en_GB-alan-high")
    except Exception as e:
        print(f"Error initializing Piper: {e}", file=sys.stderr)
        print("Make sure the en_GB-alan-high model is available.", file=sys.stderr)
        sys.exit(1)
    
    # Generate each track
    for i, track in enumerate(TRACKS, start=1):
        track_num = f"{i:02d}"
        title = track["title"]
        text = track["text"]
        
        print(f"Generating track {i}/10: {title}")
        
        # Temporary WAV file path
        wav_path = ALBUM_DIR / f"{track_num}. {title}.wav"
        # Final FLAC file path
        flac_path = ALBUM_DIR / f"{track_num}. {title}.flac"
        
        try:
            # Generate WAV file using piper-tts
            piper.synthesize(text, str(wav_path))
            
            # Convert WAV to FLAC using ffmpeg
            subprocess.run(
                [
                    "ffmpeg",
                    "-i", str(wav_path),
                    "-c:a", "flac",
                    "-y",  # Overwrite output file if it exists
                    str(flac_path),
                ],
                check=True,
                capture_output=True,
            )
            
            # Remove the temporary WAV file
            wav_path.unlink()
            
            # Add metadata to FLAC file
            audio = FLAC(str(flac_path))
            audio["artist"] = "The Announcers"
            audio["album"] = "First Test"
            audio["date"] = "2025"
            audio["title"] = title
            audio["tracknumber"] = str(i)
            audio.save()
            
            print(f"  ✓ Generated {flac_path.name}")
            
        except subprocess.CalledProcessError as e:
            print(f"Error processing track {i}: {e}", file=sys.stderr)
            if e.stderr:
                print(f"ffmpeg error: {e.stderr.decode()}", file=sys.stderr)
            sys.exit(1)
        except Exception as e:
            print(f"Error processing track {i}: {e}", file=sys.stderr)
            sys.exit(1)
    
    print(f"\n✓ Collection generated successfully in {ALBUM_DIR}")
    print(f"  Total tracks: {len(TRACKS)}")


if __name__ == "__main__":
    generate_collection()

