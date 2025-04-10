use lazy_static::lazy_static;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

// --- Regex Definitions --- (Keep as before)
lazy_static! {
    static ref SE_RE: Regex = Regex::new(r"(?i)S(\d{1,3})E(\d{1,3})").unwrap();
    static ref E_RE: Regex = Regex::new(r"(?i)E(\d{1,3})").unwrap();
    static ref SUFFIX_RE: Regex = Regex::new(r"-(?:[^-]+)(\[[^\]]+\])$").unwrap();
}

// --- Structs --- (Keep ParsedInfo as before)
#[derive(Debug, Clone)]
struct ParsedInfo {
    original_path: PathBuf,
    original_filename: String,
    extension: String,
    show_name_part: Option<String>,
    season_prefix_part: Option<String>,
    episode_number_part: Option<String>,
    remainder_part: Option<String>,
    needs_user_input: bool,
}

#[derive(Debug)]
enum ParseError {
    NotAFile,
    NoFileName,
    // Could add more specific errors if needed
}

// --- Helper Functions --- (Keep clean_segment, parse_filename, get_dir_name, prompt_user as before)
// ... (paste the existing helper functions here) ...
/// Cleans a string segment: replaces spaces with dots, removes multiple dots.
fn clean_segment(segment: &str) -> String {
    let mut cleaned = segment.trim().replace(' ', ".");
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    // Don't let segments start/end with dots unless it's the only char
    if cleaned != "." {
        cleaned = cleaned.trim_matches('.').to_string();
    }
    cleaned
}

/// Attempts to parse filename components.
fn parse_filename(path: &Path) -> Result<ParsedInfo, ParseError> {
    if !path.is_file() {
        return Err(ParseError::NotAFile);
    }

    let original_filename = path
        .file_name()
        .ok_or(ParseError::NoFileName)?
        .to_string_lossy()
        .into_owned();

    let mut stem = path
        .file_stem()
        .map_or(String::new(), |s| s.to_string_lossy().into_owned());

    let extension = path
        .extension()
        .map_or(String::new(), |e| e.to_string_lossy().into_owned());

    // 1. Remove suffix like -Name[Location] first
    if let Some(captures) = SUFFIX_RE.captures(&stem) {
        if let Some(mat) = captures.get(0) {
            stem.truncate(mat.start());
            stem = stem.trim_end().to_string(); // Remove potential trailing space
        }
    }

    // 2. Try to find SxxExx
    let mut show_name_part: Option<String> = None;
    let mut season_prefix_part: Option<String> = None;
    let mut episode_number_part: Option<String> = None;
    let mut remainder_part: Option<String> = None;
    let mut needs_user_input = false;

    if let Some(se_match) = SE_RE.find(&stem) {
        // Found SxxExx
        let potential_show = clean_segment(&stem[..se_match.start()]);
        if !potential_show.is_empty() {
            show_name_part = Some(potential_show);
        } else {
            needs_user_input = true; // Show name missing before SxxExx
        }

        // Extract Sxx and Exx using captures from the specific SxxExx regex
        if let Some(caps) = SE_RE.captures(se_match.as_str()) {
            // Format season number with leading zero if needed
            let season_num: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
            season_prefix_part = Some(format!("S{:02}", season_num));

            // Format episode number with leading zero if needed
            let episode_num: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
            episode_number_part = Some(format!("E{:02}", episode_num));
        } else {
            // This shouldn't happen if SE_RE.find succeeded, but handle defensively
            needs_user_input = true;
        }

        let potential_remainder = clean_segment(&stem[se_match.end()..]);
        if !potential_remainder.is_empty() {
            remainder_part = Some(potential_remainder);
        }
    } else {
        // SxxExx not found, mark for user input regarding Show and Season
        needs_user_input = true;
        // Still try to find Exx independently for later reconstruction
        if let Some(e_match) = E_RE.find(&stem) {
            if let Some(caps) = E_RE.captures(e_match.as_str()) {
                let episode_num: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
                episode_number_part = Some(format!("E{:02}", episode_num));

                // Attempt to derive show name from part before E## if S## was missing
                let potential_show = clean_segment(&stem[..e_match.start()]);
                if !potential_show.is_empty() {
                    show_name_part = Some(potential_show); // Use this if user doesn't provide one
                }

                // Remainder is after E##
                let potential_remainder = clean_segment(&stem[e_match.end()..]);
                if !potential_remainder.is_empty() {
                    remainder_part = Some(potential_remainder);
                }
            }
        } else {
            // Neither SxxExx nor Exx found. Consider the whole stem as potential show/remainder?
            // For now, we'll rely on user input for Show/Season, and Exx is just missing.
            // We could try setting show_name_part = clean_segment(&stem) here as a fallback.
            let potential_show = clean_segment(&stem);
            if !potential_show.is_empty() {
                show_name_part = Some(potential_show);
            }
        }
    }

    // Final check: if we think we parsed correctly but show or season is still missing, mark it.
    if show_name_part.is_none() || season_prefix_part.is_none() {
        needs_user_input = true;
    }
    // Crucial: If we need user input for Season, we MUST have an Episode number parsed
    if needs_user_input && season_prefix_part.is_none() && episode_number_part.is_none() {
        // Cannot construct SxxExx later if Exx is missing now.
        // Treat this as unparsable for renaming purposes without Exx.
        // We could potentially skip these files later. For now, keep needs_user_input=true
        // and let the second pass handle the missing Exx.
        // Reduce verbosity here now that we filter extensions first
        // println!("Warning: File '{}' is missing Season and Episode identifiers (SxxExx). It might be skipped or require manual intervention if confirmed.", original_filename);
    }

    Ok(ParsedInfo {
        original_path: path.to_path_buf(),
        original_filename,
        extension,
        show_name_part,
        season_prefix_part,
        episode_number_part,
        remainder_part,
        needs_user_input,
    })
}

/// Gets the directory name (last component) of a path, if possible.
fn get_dir_name(path: &Path) -> Option<String> {
    path.file_name().and_then(OsStr::to_str).map(str::to_string)
}

/// Prompts the user for input with an optional default value.
fn prompt_user(prompt_text: &str, default_value: Option<&str>) -> io::Result<String> {
    match default_value {
        Some(def) if !def.is_empty() => print!("{} [Default: {}]: ", prompt_text, def),
        _ => print!("{}: ", prompt_text),
    }
    io::stdout().flush()?; // Ensure prompt is displayed before reading input

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed_input = input.trim();

    if trimmed_input.is_empty() && default_value.is_some() {
        Ok(default_value.unwrap().to_string())
    } else {
        Ok(trimmed_input.to_string())
    }
}

// --- Main Function ---
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_directory = env::current_dir()?;
    println!("Scanning directory: {}", target_directory.display());

    let script_path = env::current_exe().ok(); // To avoid renaming the script

    // --- Define Allowed Video Extensions (Lowercase) ---
    let video_extensions: HashSet<String> = [
        "mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "mpeg", "mpg", "ts", "m2ts",
        "vob",
        // Add any other extensions you commonly encounter
    ]
    .iter()
    .map(|&s| s.to_lowercase())
    .collect();

    // --- Get Default Names from Directory Structure --- (Keep as before)
    let parent_dir = target_directory.parent();
    let grandparent_dir = parent_dir.and_then(|p| p.parent());
    let default_season_dir_name = parent_dir.and_then(get_dir_name);
    let default_show_dir_name = grandparent_dir.and_then(get_dir_name);

    let mut parsed_files_info: Vec<ParsedInfo> = Vec::new();
    let mut all_paths_in_dir: HashSet<PathBuf> = HashSet::new();
    let mut any_file_needs_input = false;

    // --- Pass 1: Parse all files ---
    println!("Filtering for video files: {:?}", video_extensions); // Optional: Show which extensions are checked
    for entry_result in fs::read_dir(&target_directory)? {
        let entry = entry_result?;
        let path = entry.path();
        all_paths_in_dir.insert(path.clone()); // Store all paths found

        // Skip self
        if let Some(script) = &script_path {
            if path == *script {
                continue;
            }
        }

        // --- Add Extension Check Here ---
        if path.is_file() {
            let extension = path
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_lowercase)
                .unwrap_or_default(); // Get extension, make lowercase

            if !video_extensions.contains(&extension) {
                // println!("Skipping non-video file: {}", path.display()); // Optional: Verbose logging
                continue; // Skip this file if extension is not in the list
            }

            // --- Only proceed if it's a video file ---
            match parse_filename(&path) {
                Ok(info) => {
                    // Only print warning if it's a video file that needs input
                    if info.needs_user_input
                        && info.season_prefix_part.is_none()
                        && info.episode_number_part.is_none()
                    {
                        println!(
                            "Warning: Video file '{}' is missing Season and Episode identifiers (SxxExx).",
                            info.original_filename
                        );
                    }

                    if info.needs_user_input {
                        any_file_needs_input = true;
                    }
                    parsed_files_info.push(info);
                }
                Err(ParseError::NotAFile) => { /* Should not happen here as we check is_file */ }
                Err(e) => eprintln!("Warning: Could not parse '{}': {:?}", path.display(), e),
            }
        }
        // --- End Extension Check ---
    }

    if parsed_files_info.is_empty() {
        println!("No eligible video files found to process in this directory.");
        return Ok(());
    }

    // --- User Input Phase (if needed) --- (Keep as before)
    // ... (paste the existing User Input Phase here) ...
    let mut global_show_name: Option<String> = None;
    let mut global_season_prefix: Option<String> = None; // Expecting "S01", "S12" etc.

    if any_file_needs_input {
        println!("\nSome video files lack Show Name or Season info (Sxx) in the filename.");

        let user_show_name = prompt_user(
            "Enter Show Name for these files",
            default_show_dir_name.as_deref(),
        )?;
        if !user_show_name.is_empty() {
            global_show_name = Some(clean_segment(&user_show_name));
        } else {
            println!(
                "No Show Name provided, files needing it might be skipped or use partial names."
            );
        }

        let user_season_str = prompt_user(
            "Enter Season Number (e.g., 1, 02, 15) for these files",
            default_season_dir_name.as_deref(), // Default might be "Season 01" or just "1"
        )?;

        // Attempt to parse season number from various inputs
        let cleaned_season_input =
            user_season_str.trim_start_matches(|c: char| !c.is_ascii_digit()); // Remove leading non-digits like "Season "
        if let Ok(num) = cleaned_season_input.parse::<u32>() {
            global_season_prefix = Some(format!("S{:02}", num));
        } else {
            println!(
                "Could not parse Season Number '{}'. Files needing it will be skipped.",
                user_season_str
            );
            // Set any_file_needs_input to false effectively, as we can't proceed for those files
            any_file_needs_input = false; // Prevent trying to rename files that needed this input
        }
    }

    // --- Pass 2: Construct Final Names & Prepare Renames --- (Keep as before)
    // ... (paste the existing Pass 2 logic here) ...
    let mut proposed_renames: HashMap<PathBuf, PathBuf> = HashMap::new();

    for info in parsed_files_info {
        let mut final_show: Option<String> = info.show_name_part.clone();
        let mut final_season: Option<String> = info.season_prefix_part.clone();
        let final_episode: Option<String> = info.episode_number_part.clone(); // Must exist if season was missing
        let final_remainder: Option<String> = info.remainder_part.clone();
        let final_extension: String = info.extension.clone();

        if info.needs_user_input && any_file_needs_input {
            // Check any_file_needs_input again in case user failed to provide valid input
            // Apply global overrides if available
            if global_show_name.is_some() {
                final_show = global_show_name.clone();
            }
            if global_season_prefix.is_some() {
                final_season = global_season_prefix.clone();
            }

            // Critical check: Can we form "SxxExx"?
            if final_season.is_none() || final_episode.is_none() {
                println!(
                    "Skipping '{}': Cannot determine final Season/Episode ({} / {}) after prompts.",
                    info.original_filename,
                    final_season.as_deref().unwrap_or("Missing"),
                    final_episode.as_deref().unwrap_or("Missing")
                );
                continue; // Skip this file
            }
        }

        // Construct the new stem
        let mut new_stem_parts: Vec<String> = Vec::new();
        if let Some(show) = final_show {
            if !show.is_empty() {
                new_stem_parts.push(show);
            } else {
                println!(
                    "Warning: Skipping '{}' due to empty show name component.",
                    info.original_filename
                );
                continue;
            }
        } else {
            println!(
                "Warning: Skipping '{}' due to missing show name component.",
                info.original_filename
            );
            continue;
        }

        if let Some(season) = final_season {
            if let Some(episode) = final_episode {
                new_stem_parts.push(format!("{}{}", season, episode));
            } else {
                println!(
                    "Warning: Skipping '{}' due to missing episode component.",
                    info.original_filename
                );
                continue;
            }
        } else {
            println!(
                "Warning: Skipping '{}' due to missing season component.",
                info.original_filename
            );
            continue;
        }

        if let Some(rem) = final_remainder {
            if !rem.is_empty() {
                new_stem_parts.push(rem);
            }
        }

        let new_stem = new_stem_parts.join(".");

        // Reassemble the filename
        let new_filename_str = if final_extension.is_empty() {
            new_stem
        } else {
            format!("{}.{}", new_stem, final_extension)
        };

        // Check if filename actually changed
        if new_filename_str != info.original_filename {
            let parent_dir = info
                .original_path
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let new_path = parent_dir.join(new_filename_str);
            // Check if the *generated* new path is different from the original
            if new_path != info.original_path {
                proposed_renames.insert(info.original_path, new_path);
            }
        }
    }

    // --- Display proposed changes --- (Keep as before)
    // ... (paste the existing Display logic here) ...
    if proposed_renames.is_empty() {
        println!("\nNo files need renaming based on the current rules and inputs.");
        return Ok(());
    }

    // --- Display proposed changes --- (Same as before)
    println!("\nProposed renames:");
    println!("--------------------");
    let max_len_old = proposed_renames
        .keys()
        .filter_map(|p| p.file_name())
        .map(|n| n.len())
        .max()
        .unwrap_or(0);

    for (old, new) in &proposed_renames {
        let old_name = old.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        let new_name = new.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        println!("{:<width$} -> {}", old_name, new_name, width = max_len_old);
    }
    println!("--------------------");

    // --- Conflict Checking --- (Keep as before)
    // ... (paste the existing Conflict Checking logic here) ...
    let mut potential_conflicts = Vec::new();
    let target_filenames: HashSet<&PathBuf> = proposed_renames.values().collect();

    for new_target_path_ref in &target_filenames {
        let target_path: &PathBuf = *new_target_path_ref;
        if all_paths_in_dir.contains(target_path) && !proposed_renames.contains_key(target_path) {
            potential_conflicts.push(format!(
                "Target '{}' already exists and is not being renamed.",
                target_path
                    .file_name()
                    .map_or("?", |n| n.to_str().unwrap_or("?"))
            ));
        }
    }

    let mut target_counts: HashMap<&PathBuf, usize> = HashMap::new();
    for target_path in proposed_renames.values() {
        *target_counts.entry(target_path).or_insert(0) += 1;
    }

    for (target_path, count) in target_counts {
        if count > 1 {
            let conflicting_originals: Vec<String> = proposed_renames
                .iter()
                .filter(|&(_, new)| new == target_path)
                .map(|(old, _)| {
                    old.file_name()
                        .map_or("?".to_string(), |n| n.to_string_lossy().into_owned())
                })
                .collect();
            potential_conflicts.push(format!(
                "Multiple files would be renamed to '{}': {:?}",
                target_path
                    .file_name()
                    .map_or("?", |n| n.to_str().unwrap_or("?")),
                conflicting_originals
            ));
        }
    }

    if !potential_conflicts.is_empty() {
        eprintln!("\nWarning: Potential conflicts detected!");
        for conflict in potential_conflicts {
            eprintln!("- {}", conflict);
        }
        eprintln!("Please resolve conflicts before proceeding.");
        process::exit(1);
    }

    // --- Confirmation and Renaming --- (Keep as before)
    // ... (paste the existing Confirmation/Renaming logic here) ...
    print!("\nProceed with renaming? (yes/no): ");
    io::stdout().flush()?;
    let mut confirmation = String::new();
    io::stdin().read_line(&mut confirmation)?;

    if confirmation.trim().to_lowercase() == "yes" {
        println!("\nRenaming files...");
        let mut success_count = 0;
        let mut error_count = 0;

        for (old, new) in &proposed_renames {
            match fs::rename(old, new) {
                Ok(_) => {
                    println!(
                        "Renamed: '{}' to '{}'",
                        old.file_name().map_or("?", |n| n.to_str().unwrap_or("?")),
                        new.file_name().map_or("?", |n| n.to_str().unwrap_or("?"))
                    );
                    success_count += 1;
                }
                Err(e) => {
                    eprintln!(
                        "Error renaming '{}' to '{}': {}",
                        old.file_name().map_or("?", |n| n.to_str().unwrap_or("?")),
                        new.file_name().map_or("?", |n| n.to_str().unwrap_or("?")),
                        e
                    );
                    error_count += 1;
                }
            }
        }
        println!("--------------------");
        println!(
            "Renaming complete. {} succeeded, {} failed.",
            success_count, error_count
        );
    } else {
        println!("Renaming cancelled by user.");
    }

    Ok(())
}
