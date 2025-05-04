# TV Episode Renamer

A command-line tool written in Rust to rename TV episode files based on common patterns, applying specific formatting and capitalization rules.

## Features

*   **Cleans Filenames:** Removes common release group suffixes (e.g., `-Group[Source]`).
*   **Standardizes Separators:** Replaces spaces and multiple dots with single dots.
*   **Formats Season/Episode:** Identifies and formats season/episode numbers as `SxxExx` (e.g., `S01E02`), ensuring 'S' and 'E' are uppercase. Handles missing season numbers (`Exx` only) by prompting the user.
*   **Title Case Capitalization:** Capitalizes the show name part using title case rules.
    *   Words like "the", "of", "and" remain lowercase unless they are the first word.
*   **Remainder Preservation:** Keeps any text *after* the SxxExx part (like resolution, codec info) lowercase and dot-separated.
*   **Extension Preservation:** Keeps the original file extension and its case.
*   **Interactive Prompts:** If the show name or season cannot be reliably parsed from the filename, it prompts the user for input, suggesting defaults based on parent directory names.
*   **Video File Filtering:** Processes only files with common video extensions (mkv, mp4, avi, etc. - list is hardcoded).
*   **Conflict Detection:** Checks for potential filename collisions before renaming and aborts if conflicts are found.
*   **Confirmation:** Displays proposed renames and requires user confirmation (y/yes) before proceeding.

## Example

**Before:**
`sun.wars.tales.of.the.oveworld.s01e02.1080p.web.h264-sylix[EZTVx.to].mkv`

**After:**
`Sun.Wars.Tales.of.the.Overworld.S01E02.1080p.web.h264.mkv`

## Prerequisites

*   [Rust](https://www.rust-lang.org/tools/install) (includes Cargo, the Rust package manager)

## Building

1.  Clone the repository:
    ```bash
    git clone git@github.com:shaneturner/rename-episodes.git
    cd rename-episodes
    ```
2.  Build the release executable:
    ```bash
    cargo build --release
    ```
    The executable will be located at `target/release/rename-episodes` (or `target\release\rename-episodes.exe` on Windows).

## Usage

1.  Copy or move the compiled `rename-episodes` executable to a location in your system's PATH, or run it directly using its full path.
2.  Navigate your terminal to the directory containing the TV episode files you want to rename.
    ```bash
    cd /path/to/your/media/Show Name/Season 01/
    ```
3.  Run the executable:
    ```bash
    /path/to/rename-episodes
    ```
    (Or just `rename-episodes` if it's in your PATH).

4.  The script will:
    *   Scan the current directory for video files.
    *   Parse filenames and identify potential renames.
    *   If show names or season numbers are missing, it will prompt you for input (using parent directory names as suggestions if available).
    *   Display a list of proposed renames.
    *   Check for filename conflicts.
    *   Ask for confirmation (`y/n`) before applying any changes.

## Configuration

*   **Video Extensions:** The list of recognized video file extensions is hardcoded in `main.rs`. You can modify the `video_extensions` `HashSet` if needed.
*   **Capitalization Exceptions:** The words excluded from title capitalization ("the", "of", "and") are hardcoded in the `capitalize_title_case` function.

## Dependencies

*   [regex](https://crates.io/crates/regex): For filename parsing.
*   [lazy_static](https://crates.io/crates/lazy_static): For initializing regex patterns efficiently.

## License

MIT
