use lazy_static::lazy_static;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

lazy_static! {
    // Regex to find SxxExx or SxxExxx patterns, case-insensitive. Captures season and episode numbers.
    static ref SE_RE: Regex = Regex::new(r"(?i)S(\d{1,3})E(\d{1,3})").unwrap();
    // Regex to find Exx or Exxx patterns (if Sxx is missing), case-insensitive. Captures episode number.
    static ref E_RE: Regex = Regex::new(r"(?i)E(\d{1,3})").unwrap();
    // Regex to find common suffix patterns like "-GroupName[Source]" at the end of the filename stem.
    static ref SUFFIX_RE: Regex = Regex::new(r"-(?:[^-]+)(\[[^\]]+\])$").unwrap();
}

#[derive(Debug, Clone)]
struct ParsedInfo {
    original_path: PathBuf,
    original_filename: String,
    extension: String,
    show_name_part: Option<String>, // Cleaned, lowercase, dot-separated part before SxxExx
    season_prefix_part: Option<String>, // Formatted as "Sxx"
    episode_number_part: Option<String>, // Formatted as "Exx"
    remainder_part: Option<String>, // Cleaned, lowercase, dot-separated part after SxxExx
    needs_user_input: bool,         // Flag if show name or season needs to be derived/confirmed
}

#[derive(Debug)]
enum ParseError {
    NotAFile,
    NoFileName,
}

/// Cleans a string segment: converts to lowercase, replaces spaces with dots, removes multiple dots.
fn clean_segment(segment: &str) -> String {
    let mut cleaned = segment.trim().replace(' ', ".");
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    if cleaned != "." {
        cleaned = cleaned.trim_matches('.').to_string();
    }
    cleaned.to_lowercase()
}

/// Capitalizes words in a dot-separated string according to Title Case rules, skipping specific exceptions.
fn capitalize_title_case(text: &str) -> String {
    let exceptions: HashSet<&str> = ["the", "of", "and"].iter().cloned().collect();

    text.split('.')
        .enumerate()
        .map(|(index, word)| {
            if word.is_empty() {
                String::new()
            } else if index == 0 || !exceptions.contains(word) {
                // Capitalize the first word OR any word not in exceptions
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            } else {
                // Keep exception words lowercase (unless first word)
                word.to_string()
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<String>>()
        .join(".")
}

/// Attempts to parse filename components (show, season, episode, remainder, extension).
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

    // 1. Remove suffix like -GroupName[Source] first
    if let Some(captures) = SUFFIX_RE.captures(&stem) {
        if let Some(mat) = captures.get(0) {
            stem.truncate(mat.start());
            stem = stem.trim_end().to_string();
        }
    }

    // 2. Try to find SxxExx
    let mut show_name_part: Option<String> = None;
    let mut season_prefix_part: Option<String> = None;
    let mut episode_number_part: Option<String> = None;
    let mut remainder_part: Option<String> = None;
    let mut needs_user_input = false;

    if let Some(se_match) = SE_RE.find(&stem) {
        let potential_show = clean_segment(&stem[..se_match.start()]);
        if !potential_show.is_empty() {
            show_name_part = Some(potential_show);
        } else {
            needs_user_input = true; // Show name missing before SxxExx
        }

        if let Some(caps) = SE_RE.captures(se_match.as_str()) {
            let season_num: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
            season_prefix_part = Some(format!("S{:02}", season_num)); // Force uppercase S

            let episode_num: u32 = caps.get(2).unwrap().as_str().parse().unwrap_or(0);
            episode_number_part = Some(format!("E{:02}", episode_num)); // Force uppercase E
        } else {
            // This case should be unlikely if SE_RE.find matched, but handle defensively
            needs_user_input = true;
        }

        let potential_remainder = clean_segment(&stem[se_match.end()..]);
        if !potential_remainder.is_empty() {
            remainder_part = Some(potential_remainder);
        }
    } else {
        // SxxExx not found, will need input for Season
        needs_user_input = true;
        // Still try to find Exx independently for later reconstruction
        if let Some(e_match) = E_RE.find(&stem) {
            if let Some(caps) = E_RE.captures(e_match.as_str()) {
                let episode_num: u32 = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
                episode_number_part = Some(format!("E{:02}", episode_num)); // Force uppercase E

                let potential_show = clean_segment(&stem[..e_match.start()]);
                if !potential_show.is_empty() {
                    show_name_part = Some(potential_show); // May be overridden by user input later
                }

                let potential_remainder = clean_segment(&stem[e_match.end()..]);
                if !potential_remainder.is_empty() {
                    remainder_part = Some(potential_remainder);
                }
            }
        } else {
            // Neither SxxExx nor Exx found. Treat the whole stem as potential show name.
            let potential_show = clean_segment(&stem);
            if !potential_show.is_empty() {
                show_name_part = Some(potential_show);
            }
        }
    }

    // If essential info (Show or Season) is missing after parsing, confirm user input is needed.
    if show_name_part.is_none() || season_prefix_part.is_none() {
        needs_user_input = true;
    }

    // If user input is needed for Season, we *must* have found an Episode number.
    if needs_user_input && season_prefix_part.is_none() && episode_number_part.is_none() {
        // A warning will be printed later if this is a video file.
    }

    Ok(ParsedInfo {
        original_path: path.to_path_buf(),
        original_filename,
        extension,           // Preserve original extension case
        show_name_part,      // Store cleaned/lowercase for now
        season_prefix_part,  // Store "Sxx"
        episode_number_part, // Store "Exx"
        remainder_part,      // Store cleaned/lowercase
        needs_user_input,
    })
}

/// Gets the directory name (last component) of a path, if possible. Used for default suggestions.
fn get_dir_name(path: &Path) -> Option<String> {
    path.file_name().and_then(OsStr::to_str).map(str::to_string)
}

/// Prompts the user for input with an optional default value.
fn prompt_user(prompt_text: &str, default_value: Option<&str>) -> io::Result<String> {
    match default_value {
        Some(def) if !def.is_empty() => print!("{} [Default: {}]: ", prompt_text, def),
        _ => print!("{}: ", prompt_text),
    }
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed_input = input.trim();

    if trimmed_input.is_empty() && default_value.is_some() {
        Ok(default_value.unwrap().to_string())
    } else {
        Ok(trimmed_input.to_string())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_directory = env::current_dir()?;
    println!("Scanning directory: {}", target_directory.display());

    let script_path = env::current_exe().ok(); // To avoid renaming the script itself

    // Define common video file extensions (lowercase for comparison)
    let video_extensions: HashSet<String> = [
        "mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "mpeg", "mpg", "ts", "m2ts",
        "vob", // Add others if needed
    ]
    .iter()
    .map(|&s| s.to_lowercase())
    .collect();

    // Try to get default Show/Season names from parent/grandparent directory names
    let parent_dir = target_directory.parent();
    let grandparent_dir = parent_dir.and_then(|p| p.parent());
    let default_season_dir_name = parent_dir.and_then(get_dir_name);
    let default_show_dir_name = grandparent_dir.and_then(get_dir_name);

    let mut parsed_files_info: Vec<ParsedInfo> = Vec::new();
    let mut all_paths_in_dir: HashSet<PathBuf> = HashSet::new(); // Keep track of all items for conflict checking
    let mut any_file_needs_input = false;

    // Pass 1: Parse all relevant files and identify if user input is globally needed
    println!("Filtering for video files: {:?}", video_extensions);
    for entry_result in fs::read_dir(&target_directory)? {
        let entry = entry_result?;
        let path = entry.path();
        all_paths_in_dir.insert(path.clone());

        if let Some(script) = &script_path {
            if path == *script {
                continue; // Skip the running script
            }
        }

        if path.is_file() {
            let extension = path
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_lowercase)
                .unwrap_or_default();

            if !video_extensions.contains(&extension) {
                continue; // Skip non-video files
            }

            // Parse the video file
            match parse_filename(&path) {
                Ok(info) => {
                    // Warn if essential SxxExx info seems missing for a video file
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
                Err(ParseError::NotAFile) => {} // Should not happen due to is_file check
                Err(e) => eprintln!("Warning: Could not parse '{}': {:?}", path.display(), e),
            }
        }
    }

    if parsed_files_info.is_empty() {
        println!("No eligible video files found to process in this directory.");
        return Ok(());
    }

    // User Input Phase: Get global Show/Season if any file required it
    let mut global_show_name: Option<String> = None; // Will store cleaned/lowercase version
    let mut global_season_prefix: Option<String> = None; // Will store "Sxx"

    if any_file_needs_input {
        println!("\nSome video files lack Show Name or Season info (Sxx) in the filename.");

        let user_show_name = prompt_user(
            "Enter Show Name for these files",
            default_show_dir_name.as_deref(),
        )?;
        if !user_show_name.is_empty() {
            global_show_name = Some(clean_segment(&user_show_name)); // Clean the input
        } else {
            println!(
                "No Show Name provided, files needing it might be skipped or use partial names."
            );
        }

        let user_season_str = prompt_user(
            "Enter Season Number (e.g., 1, 02, 15) for these files",
            default_season_dir_name.as_deref(),
        )?;

        // Attempt to parse season number and format correctly ("Sxx")
        let cleaned_season_input =
            user_season_str.trim_start_matches(|c: char| !c.is_ascii_digit());
        if let Ok(num) = cleaned_season_input.parse::<u32>() {
            global_season_prefix = Some(format!("S{:02}", num)); // Ensure uppercase S
        } else {
            println!(
                "Could not parse Season Number '{}'. Files needing it will be skipped.",
                user_season_str
            );
            // If we can't get a valid season, disable applying global overrides for files that needed it
            any_file_needs_input = false;
        }
    }

    // Pass 2: Construct Final Names & Prepare Renames
    let mut proposed_renames: HashMap<PathBuf, PathBuf> = HashMap::new();

    for info in parsed_files_info {
        // Start with parsed info, potentially override with global input
        let mut final_show = info.show_name_part.clone();
        let mut final_season = info.season_prefix_part.clone();
        let final_episode = info.episode_number_part.clone();
        let final_remainder = info.remainder_part.clone();
        let final_extension = info.extension.clone();

        // Apply global overrides only if input was needed for this file and successfully provided
        if info.needs_user_input && any_file_needs_input {
            if let Some(global_show) = &global_show_name {
                final_show = Some(global_show.clone());
            }
            if let Some(global_season) = &global_season_prefix {
                final_season = Some(global_season.clone());
            }

            // Critical check: Can we form "SxxExx" after potential overrides?
            if final_season.is_none() || final_episode.is_none() {
                println!(
                    "Skipping '{}': Cannot determine final Season/Episode ({} / {}) after prompts.",
                    info.original_filename,
                    final_season.as_deref().unwrap_or("Missing"),
                    final_episode.as_deref().unwrap_or("Missing")
                );
                continue; // Skip this file if essential parts are missing
            }
        }

        // Construct the new filename stem piece by piece
        let mut new_stem_parts: Vec<String> = Vec::new();

        // 1. Show Name (Apply Title Case)
        if let Some(show) = final_show {
            if !show.is_empty() {
                new_stem_parts.push(capitalize_title_case(&show));
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

        // 2. Season and Episode (Already formatted Sxx and Exx)
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

        // 3. Remainder (Keep cleaned/lowercase)
        if let Some(rem) = final_remainder {
            if !rem.is_empty() {
                new_stem_parts.push(rem);
            }
        }

        let new_stem = new_stem_parts.join(".");

        // Reassemble the full filename, preserving original extension case
        let new_filename_str = if final_extension.is_empty() {
            new_stem
        } else {
            format!("{}.{}", new_stem, final_extension)
        };

        // Check if the filename actually changed
        if new_filename_str != info.original_filename {
            let parent = info
                .original_path
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let new_path = parent.join(new_filename_str);

            // Check if the *path* actually changed (it might not if only case changed on case-insensitive FS)
            // We rely on the string comparison above mostly, but add proposed rename only if distinct paths.
            if new_path != info.original_path {
                proposed_renames.insert(info.original_path.clone(), new_path);
            }
        }
    }

    // Display proposed changes
    if proposed_renames.is_empty() {
        println!("\nNo files need renaming based on the current rules and inputs.");
        return Ok(());
    }

    println!("\nProposed renames:");
    println!("--------------------");
    let max_len_old = proposed_renames
        .keys()
        .filter_map(|p| p.file_name())
        .map(|n| n.len())
        .max()
        .unwrap_or(0);

    // Sort for consistent display order
    let mut sorted_renames: Vec<_> = proposed_renames.iter().collect();
    sorted_renames.sort_by(|(old_a, _), (old_b, _)| old_a.cmp(old_b));

    for (old, new) in &sorted_renames {
        // Borrow here for display
        let old_name = old.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        let new_name = new.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        println!("{:<width$} -> {}", old_name, new_name, width = max_len_old);
    }
    println!("--------------------");

    // Conflict Checking
    let mut potential_conflicts = Vec::new();
    let target_filenames: HashSet<&PathBuf> = proposed_renames.values().collect(); // Targets being renamed TO

    // Check if a target filename already exists in the directory *and* is not itself being renamed from
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

    // Check if multiple files are being renamed TO the same target filename
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
        process::exit(1); // Abort due to conflicts
    }

    // Confirmation and Renaming
    print!(
        "\nProceed with renaming {} file(s)? (y/n) [default: n]: ",
        proposed_renames.len()
    );
    io::stdout().flush()?;
    let mut confirmation = String::new();
    io::stdin().read_line(&mut confirmation)?;

    let trimmed_confirmation = confirmation.trim().to_lowercase();

    if trimmed_confirmation == "y" || trimmed_confirmation == "yes" {
        println!("\nRenaming files...");
        let mut success_count = 0;
        let mut error_count = 0;

        // Consume the map for the renaming process, using the sorted order
        let mut sorted_renames_for_action: Vec<_> = proposed_renames.into_iter().collect();
        sorted_renames_for_action.sort_by(|(old_a, _), (old_b, _)| old_a.cmp(old_b));

        for (old, new) in sorted_renames_for_action {
            // Iterate over owned values now
            match fs::rename(&old, &new) {
                // Borrow paths for the rename operation
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
        println!("Renaming cancelled.");
    }

    Ok(())
}
