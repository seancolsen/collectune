"""Where a track's file sits within the collection.

Shared by the two generators beside it. The database import joins the
definition to the scanned rows on the file path, so the script that *writes*
the files and the script that *looks them up* have to agree on that path
exactly — one rule, in one place, rather than two that drift.

This is a plain module rather than a uv script, and so has no dependencies of
its own.
"""

# Characters that would otherwise steer a title out of its album's directory.
# Everything else in a title is left alone — the collection is meant to exercise
# the app's handling of real-world names, punctuation and all.
PATH_SUBSTITUTIONS = str.maketrans({"/": "-", "\\": "-"})


def relative_path(template: str, album: dict, track: dict) -> str:
    """Render a track's path relative to the collection root."""
    return template.format(
        album_artist=album["album_artist"].translate(PATH_SUBSTITUTIONS),
        album=album["title"].translate(PATH_SUBSTITUTIONS),
        track_number=track["number"],
        title=track["title"].translate(PATH_SUBSTITUTIONS),
    )
