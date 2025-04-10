use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Write}; // Import Write trait for flush
use std::path::{Path, PathBuf};
use std::process;

// --- Configuration ---
// Regex to find the pattern: a hyphen, followed by non-hyphen/bracket chars (name),
// then brackets containing any characters, anchored to the end of the *stem*.
const SUFFIX_REGEX_PATTERN: &str = r"-(?:[^-]+)(\[[^\]]+\])$";

fn generate_new_filename(
    old_path: &Path,
    suffix_re: &Regex,
    script_path: Option<&Path>,
) -> Option<PathBuf> {
    // Skip directories
    if !old_path.is_file() {
        return None;
    }

    // Skip the script itself if its path is known
    if let Some(script) = script_path {
        if old_path == script {
            // More robust check comparing canonical paths might be needed in complex scenarios
            // but comparing the direct PathBufs usually works for simple cases.
            return None;
        }
    }

    let original_filename = match old_path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(), // Handle potential non-UTF8
        None => return None, // Should not happen for files in a dir listing
    };

    let stem = match old_path.file_stem() {
        Some(s) => s.to_string_lossy().into_owned(),
        None => String::new(), // Handle files like ".bashrc" (no stem)
    };

    let extension = old_path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default(); // Use empty string if no extension

    let mut new_stem = stem.clone(); // Start with the original stem

    // --- 1. Remove the "-Name[Location]" suffix ---
    if let Some(captures) = suffix_re.captures(&new_stem) {
        // Get the start index of the whole match (group 0)
        if let Some(mat) = captures.get(0) {
            new_stem.truncate(mat.start());
            new_stem = new_stem.trim_end().to_string(); // Remove potential trailing space
        }
    }

    // --- 2. Replace all spaces with dots ---
    if new_stem.contains(' ') {
        new_stem = new_stem.replace(' ', ".");
    }

    // --- 3. Clean up potential multiple dots ---
    while new_stem.contains("..") {
        new_stem = new_stem.replace("..", ".");
    }

    // --- 4. Clean up leading/trailing dots (unless it's a hidden file) ---
    if !new_stem.starts_with('.') {
        // Don't trim dot if it's the *only* thing or starts hidden file
        new_stem = new_stem.trim_matches('.').to_string();
    }

    // Avoid empty stem after processing
    if new_stem.is_empty() && !original_filename.starts_with('.') {
        eprintln!(
            "Warning: Skipping '{}' as processing resulted in empty name.",
            original_filename
        );
        return None; // Or decide how to handle this case
    }

    // --- 5. Reassemble the filename ---
    let new_filename_str = if extension.is_empty() {
        new_stem // No dot if no extension
    } else {
        format!("{}.{}", new_stem, extension)
    };

    // Only return if the name actually changed
    if new_filename_str != original_filename {
        // Construct the new path relative to the original path's parent
        let parent_dir = old_path.parent().unwrap_or_else(|| Path::new("."));
        Some(parent_dir.join(new_filename_str))
    } else {
        None // No change needed
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_directory = env::current_dir()?;
    println!("Scanning directory: {}", target_directory.display());

    // Attempt to get the script's own path to avoid renaming it
    let script_path = env::current_exe().ok(); // This might fail

    // Compile the regex once
    let suffix_re = Regex::new(SUFFIX_REGEX_PATTERN)?;

    let mut proposed_renames: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut all_paths_in_dir: HashSet<PathBuf> = HashSet::new(); // To check for existing files

    // --- First pass: Read directory and generate potential new names ---
    for entry_result in fs::read_dir(&target_directory)? {
        let entry = entry_result?;
        let old_path = entry.path();
        all_paths_in_dir.insert(old_path.clone()); // Store all paths found

        if old_path.is_file() {
            if let Some(new_path) =
                generate_new_filename(&old_path, &suffix_re, script_path.as_deref())
            {
                if old_path != new_path {
                    // Ensure there's actually a change
                    proposed_renames.insert(old_path, new_path);
                }
            }
        }
    }

    if proposed_renames.is_empty() {
        println!("No files need renaming according to the specified format.");
        return Ok(());
    }

    // --- Display proposed changes ---
    println!("\nProposed renames:");
    println!("--------------------");
    let max_len_old = proposed_renames
        .keys()
        .map(|p| p.file_name().map_or(0, |n| n.len()))
        .max()
        .unwrap_or(0);

    for (old, new) in &proposed_renames {
        let old_name = old.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        let new_name = new.file_name().map_or("?", |n| n.to_str().unwrap_or("?"));
        println!("{:<width$} -> {}", old_name, new_name, width = max_len_old);
    }
    println!("--------------------");

    // --- Check for potential conflicts ---
    let mut potential_conflicts = Vec::new();
    // target_filenames collects the VALUES (&PathBuf) from the proposed_renames map
    let target_filenames: HashSet<&PathBuf> = proposed_renames.values().collect();

    // 1. Check if a new name already exists and ISN'T one of the files being renamed
    // Iterate over the references (&PathBuf) stored in target_filenames
    for new_target_path_ref in &target_filenames {
        // new_target_path_ref is &&PathBuf
        // We need to pass a &PathBuf or &Path to contains/contains_key.
        // Dereferencing new_target_path_ref gives &PathBuf.
        let target_path: &PathBuf = *new_target_path_ref; // Now target_path is &PathBuf

        // contains/contains_key take &Q where K: Borrow<Q>.
        // Here K is PathBuf. Passing target_path (&PathBuf) means Q is PathBuf.
        // PathBuf: Borrow<PathBuf> is satisfied.
        if all_paths_in_dir.contains(target_path) && !proposed_renames.contains_key(target_path) {
            // Check if the TARGET exists AND the existing file (the target) is NOT itself slated for rename
            potential_conflicts.push(format!(
                "Target '{}' already exists and is not being renamed.",
                target_path
                    .file_name()
                    .map_or("?", |n| n.to_str().unwrap_or("?"))
            ));
        }
    }

    // 2. Check if multiple files would be renamed to the SAME target name
    // This part remains the same as it iterates over proposed_renames.values() directly
    let mut target_counts: HashMap<&PathBuf, usize> = HashMap::new();
    for target_path in proposed_renames.values() {
        // target_path is &PathBuf here
        *target_counts.entry(target_path).or_insert(0) += 1;
    }

    for (target_path, count) in target_counts {
        // target_path is &PathBuf here
        if count > 1 {
            let conflicting_originals: Vec<String> = proposed_renames
                .iter()
                .filter(|&(_, new)| new == target_path) // Compare &PathBuf == &PathBuf
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
        process::exit(1); // Exit with error status
    }

    // --- Ask for confirmation ---
    print!("\nProceed with renaming? (yes/no): ");
    io::stdout().flush()?; // Ensure prompt is displayed before reading input
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
