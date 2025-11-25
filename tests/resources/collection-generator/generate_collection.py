#!/usr/bin/env -S uv run --script
# /// script
# dependencies = [
#   "mutagen>=1.47.0",
#   "piper-tts>=1.3.0",
# ]
# ///
"""
Generate a test audio collection using piper-tts.

This script generates FLAC audio files for testing the collectune application.
It uses piper-tts to synthesize speech and converts the output to FLAC format
with appropriate metadata.
"""

import shutil
import subprocess
import sys
import wave
from pathlib import Path

from mutagen.flac import FLAC
from piper.download_voices import download_voice
from piper.voice import PiperVoice


# Define the output directory relative to this script
SCRIPT_DIR = Path(__file__).parent
COLLECTION_DIR = SCRIPT_DIR.parent / "collection"
ALBUM_DIR = COLLECTION_DIR / "The Announcers - First Test"

# Track definitions
TRACKS = [
    {"title": "Duck", "text": "One duck"},
    {"title": "Hens", "text": "Two hens"},
    {"title": "Geese", "text": "Three squawking geese"},
    {"title": "Oysters", "text": "Four limerick oysters"},
    {"title": "Porpoises", "text": "Five corpulent porpoises"},
    {"title": "Tweezers", "text": "Six pairs of Don Alverzo's tweezers"},
    {"title": "Macedonians", "text": "Seven thousand Macedonians in full battle array"},
    {"title": "Monkeys", "text": "Eight brass monkeys from the ancient sacred crypts of Egypt"},
    {"title": "Men", "text": "Nine apathetic, sympathetic, diabetic old men on roller skates, with a marked propensity toward procrastination and sloth"},
    {"title": "Denizens", "text": "Ten lyrical, spherical, diabolical denizens of the deep who all stall around the corner of the quo of the quay of the quivery, all at the same time"},
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
    print(f"Using piper-tts with model: en_GB-alan-medium")
    
    # Download voice model if needed
    voice_name = "en_GB-alan-medium"
    voice_dir = SCRIPT_DIR / ".voices"
    voice_dir.mkdir(exist_ok=True)
    
    print(f"Downloading voice model: {voice_name}")
    try:
        download_voice(voice_name, voice_dir, force_redownload=False)
    except Exception as e:
        print(f"Error downloading voice: {e}", file=sys.stderr)
        sys.exit(1)
    
    # Find the model file
    model_path = voice_dir / f"{voice_name}.onnx"
    if not model_path.exists():
        print(f"Error: Voice model not found at {model_path}", file=sys.stderr)
        sys.exit(1)
    
    # Load Piper voice
    print(f"Loading voice model: {model_path}")
    try:
        voice = PiperVoice.load(model_path)
    except Exception as e:
        print(f"Error loading voice: {e}", file=sys.stderr)
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
            # Generate audio chunks using piper-tts
            audio_chunks = voice.synthesize(text)
            
            # Write audio chunks to WAV file
            first_chunk = True
            with wave.open(str(wav_path), "wb") as wav_file:
                for chunk in audio_chunks:
                    if first_chunk:
                        # Set WAV file parameters from first chunk
                        wav_file.setnchannels(chunk.sample_channels)
                        wav_file.setsampwidth(chunk.sample_width)
                        wav_file.setframerate(chunk.sample_rate)
                        first_chunk = False
                    # Write audio data
                    wav_file.writeframes(chunk.audio_int16_bytes)
            
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

