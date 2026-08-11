//! Interactive scaffolding and editing of a configuration repository.
//!
//! `ghqc configuration init` walks through every part of a configuration
//! repository — `options.yaml`, the logo, the record template, and the
//! checklists — writing files only. Git is deliberately left to the user.
//!
//! Re-running against an existing configuration repository turns the wizard
//! into an editor: current values become prompt defaults and existing
//! checklists can be edited, renamed, or deleted.

use anyhow::{Result, anyhow, bail};
use clap::Subcommand;
use inquire::{Autocomplete, Confirm, CustomUserError, Editor, Select, Text};
use owo_colors::OwoColorize;
use serde::Serialize;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::configuration::parse_yaml_checklist;
use crate::{BUILTIN_TEMPLATE, ConfigurationOptions};

use super::section_header;

/// Starter checklists offered when authoring a new checklist.
const STARTERS: &[(&str, &str)] = &[
    (
        "General Script",
        include_str!("../templates/checklists/general_script.md"),
    ),
    (
        "Code Review",
        include_str!("../templates/checklists/code_review.md"),
    ),
    ("Report", include_str!("../templates/checklists/report.md")),
];

/// Extensions the checklist loader understands. Anything else in the checklist
/// directory is ignored by `Configuration::load_checklists`, so the wizard
/// ignores it too.
const CHECKLIST_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "yaml", "yml"];

/// Extension used for checklists the wizard writes. Markdown is the preferred
/// format: the file content becomes the checklist body verbatim, so anything
/// GitHub renders is available.
const NEW_CHECKLIST_EXTENSION: &str = "md";

/// Seed for a checklist authored from scratch. Items are `- [ ]` lines; `###`
/// headers group them into sections.
const MARKDOWN_SKELETON: &str = "### Section\n\n- [ ] [INSERT]\n";

const DONE: &str = "✅ Done";
const NEW_CHECKLIST: &str = "➕ New checklist";
const BACK: &str = "↩ Back";

/// Individually editable parts of a configuration repository.
///
/// Each maps to the same step [`configuration_init`] runs, so a targeted edit
/// and the full wizard can never disagree about how a component is written.
#[derive(Subcommand)]
pub enum ConfigurationEditCommands {
    /// Add, edit, rename, or delete checklists
    #[command(alias = "checklist")]
    Checklists,
    /// Edit the settings in options.yaml
    #[command(alias = "options.yaml")]
    Options,
    /// Replace the logo
    Logo,
    /// Write or replace the record template
    #[command(alias = "template")]
    Record,
}

/// Edit part of an existing configuration repository.
///
/// With no `command`, the components are offered as a menu that loops until
/// the user is done — naming one on the command line jumps straight to it.
///
/// The repository is resolved from `explicit` (the `--config-dir` flag) if
/// given, then the current directory when it holds an `options.yaml`, and
/// finally the configured configuration directory. Unlike
/// [`configuration_init`] this never creates a repository: editing a
/// configuration that does not exist is a mistake worth reporting.
pub fn configuration_edit(
    command: Option<ConfigurationEditCommands>,
    explicit: Option<PathBuf>,
    cwd: &Path,
    configured: &Path,
) -> Result<()> {
    let directory = resolve_edit_directory(explicit, cwd, configured)?;
    println!("📁 {}", directory.display().bold());

    match command {
        Some(command) => edit_component(&directory, command),
        None => edit_menu(&directory),
    }
}

/// Offer the editable components until the user chooses to stop.
fn edit_menu(directory: &Path) -> Result<()> {
    const CHECKLISTS: &str = "📋 Checklists";
    const OPTIONS: &str = "⚙️  Options";
    const LOGO: &str = "🖼️  Logo";
    const RECORD: &str = "📄 Record template";

    loop {
        let choice = Select::new(
            "What would you like to edit?",
            vec![CHECKLISTS, OPTIONS, LOGO, RECORD, DONE],
        )
        .prompt()
        .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

        let command = match choice {
            CHECKLISTS => ConfigurationEditCommands::Checklists,
            OPTIONS => ConfigurationEditCommands::Options,
            LOGO => ConfigurationEditCommands::Logo,
            RECORD => ConfigurationEditCommands::Record,
            _ => return Ok(()),
        };

        edit_component(directory, command)?;
        println!();
    }
}

/// Run a single step of the wizard against `directory`.
///
/// Options are re-read every time, so a component edited after `options.yaml`
/// changed uses the paths that were just set.
fn edit_component(directory: &Path, command: ConfigurationEditCommands) -> Result<()> {
    let options = load_options(&directory.join("options.yaml"))?;

    match command {
        ConfigurationEditCommands::Checklists => {
            println!("{}", section_header("Checklists"));
            let checklist_dir = directory.join(&options.checklist_directory);
            fs::create_dir_all(&checklist_dir)?;
            checklist_menu(&checklist_dir)?;
        }
        ConfigurationEditCommands::Options => {
            println!("{}", section_header("Options"));
            let mut options = options;
            prompt_options(&mut options)?;
            fs::write(directory.join("options.yaml"), options_yaml(&options)?)?;
            println!("✅ Wrote options.yaml");
        }
        ConfigurationEditCommands::Logo => {
            println!("{}", section_header("Logo"));
            prompt_logo(directory, &options)?;
        }
        ConfigurationEditCommands::Record => {
            println!("{}", section_header("Record Template"));
            prompt_record_template(directory, &options)?;
        }
    }

    Ok(())
}

/// Find the configuration repository to edit, preferring an explicit flag, then
/// the current directory, then the configured one.
fn resolve_edit_directory(
    explicit: Option<PathBuf>,
    cwd: &Path,
    configured: &Path,
) -> Result<PathBuf> {
    let is_configuration = |dir: &Path| dir.join("options.yaml").is_file();

    if let Some(explicit) = explicit {
        let explicit = if explicit.is_absolute() {
            explicit
        } else {
            cwd.join(explicit)
        };

        if !is_configuration(&explicit) {
            bail!(
                "No options.yaml in {}. Create a configuration repository there with `ghqc configuration init`",
                explicit.display()
            );
        }
        return Ok(explicit);
    }

    // Being inside the repository is the strongest signal of intent.
    if is_configuration(cwd) {
        return Ok(cwd.to_path_buf());
    }

    if is_configuration(configured) {
        return Ok(configured.to_path_buf());
    }

    bail!(
        "No configuration repository found in {} or {}. Create one with `ghqc configuration init`, or point at one with --config-dir",
        cwd.display(),
        configured.display()
    )
}

/// Run the interactive configuration wizard.
///
/// `path` skips the directory prompt when given; otherwise the directory is
/// prompted for and resolved relative to `cwd`.
pub fn configuration_init(path: Option<PathBuf>, cwd: &Path) -> Result<()> {
    println!("{}", section_header("Configuration Repository"));

    let directory = resolve_directory(path, cwd)?;
    let options_path = directory.join("options.yaml");

    let mut options = if options_path.exists() {
        let existing = load_options(&options_path)?;
        println!(
            "📁 Existing configuration found at {}",
            directory.display().bold()
        );
        if !confirm(
            &format!(
                "Edit the existing configuration at {}?",
                directory.display()
            ),
            true,
        )? {
            bail!("Aborted: nothing was changed");
        }
        existing
    } else {
        println!(
            "📁 Creating a new configuration repository at {}",
            directory.display().bold()
        );
        ConfigurationOptions::default()
    };

    println!("\n{}", section_header("Options"));
    prompt_options(&mut options)?;

    println!("\n{}", section_header("Logo"));
    prompt_logo(&directory, &options)?;

    println!("\n{}", section_header("Record Template"));
    prompt_record_template(&directory, &options)?;

    println!("\n{}", section_header("Checklists"));
    let checklist_dir = directory.join(&options.checklist_directory);
    fs::create_dir_all(&checklist_dir)?;
    checklist_menu(&checklist_dir)?;

    fs::write(&options_path, options_yaml(&options)?)?;

    println!("\n{}", section_header("Summary"));
    print_summary(&directory, &options)?;

    Ok(())
}

/// Resolve the target directory, creating it after an explicit confirmation
/// when it does not exist yet.
fn resolve_directory(path: Option<PathBuf>, cwd: &Path) -> Result<PathBuf> {
    let raw = match path {
        Some(p) => p,
        None => {
            let input = Text::new("📁 Configuration repository directory:")
                .with_default(".")
                .with_help_message("Relative paths are resolved against the current directory")
                .prompt()
                .map_err(|e| anyhow!("Input cancelled: {e}"))?;
            PathBuf::from(input.trim())
        }
    };

    let resolved = if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    };

    if resolved.exists() {
        if !resolved.is_dir() {
            bail!("{} exists but is not a directory", resolved.display());
        }
        return Ok(resolved);
    }

    println!("⚠️  {} does not exist", resolved.display().yellow());
    if !confirm("Create it?", true)? {
        bail!("Aborted: {} was not created", resolved.display());
    }
    fs::create_dir_all(&resolved)?;

    Ok(resolved)
}

fn load_options(path: &Path) -> Result<ConfigurationOptions> {
    let content = fs::read_to_string(path)?;
    serde_yaml::from_str(&content).map_err(|e| {
        anyhow!(
            "Could not parse {} as configuration options: {e}. Fix or remove the file, then retry",
            path.display()
        )
    })
}

/// Prompt for every field of `options.yaml`, pre-filled with the current value.
fn prompt_options(options: &mut ConfigurationOptions) -> Result<()> {
    options.checklist_display_name = text(
        "What should checklists be called in the UI?",
        &options.checklist_display_name,
    )?;

    let note = optional_text_with_help(
        "Note prepended to every checklist:",
        options.prepended_checklist_note.as_deref(),
        "Leave empty for no note",
    )?;
    options.prepended_checklist_note = note;

    options.include_collaborators = confirm(
        "Detect and include collaborator metadata on issues?",
        options.include_collaborators,
    )?;

    options.checklist_directory = path_text("Checklist directory:", &options.checklist_directory)?;
    options.logo_path = path_text("Logo path:", &options.logo_path)?;
    options.record_path = path_text("Record template path:", &options.record_path)?;

    options.ui_repo_refresh_rate_seconds = prompt_refresh_rate(options)?;
    options.allow_ui_config_update = prompt_allow_ui_update(options)?;

    Ok(())
}

fn prompt_refresh_rate(options: &ConfigurationOptions) -> Result<Option<u64>> {
    let current = options.ui_repo_refresh_rate_seconds.map(|s| s.to_string());

    let Some(input) = optional_text_with_help(
        "Web UI repository refresh rate in seconds:",
        current.as_deref(),
        "Leave empty to fall back to GHQC_UI_REFRESH_RATE, then 15",
    )?
    else {
        return Ok(None);
    };

    let trimmed = input.trim();
    match trimmed.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(Some(seconds)),
        _ => {
            println!("⚠️  {trimmed:?} is not a positive whole number; leaving the option unset");
            Ok(None)
        }
    }
}

fn prompt_allow_ui_update(options: &ConfigurationOptions) -> Result<Option<bool>> {
    const YES: &str = "Yes";
    const NO: &str = "No";
    const UNSET: &str = "Unset (falls back to GHQC_ALLOW_CONFIG_UPDATE, then yes)";

    let starting_cursor = match options.allow_ui_config_update {
        Some(true) => 0,
        Some(false) => 1,
        None => 2,
    };

    let choice = Select::new(
        "May the Web UI update the configuration repository?",
        vec![YES, NO, UNSET],
    )
    .with_starting_cursor(starting_cursor)
    .prompt()
    .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

    Ok(match choice {
        YES => Some(true),
        NO => Some(false),
        _ => None,
    })
}

/// Copy a logo into the repository. Optional: an empty answer keeps whatever is
/// already there (or leaves the repository without a logo).
fn prompt_logo(directory: &Path, options: &ConfigurationOptions) -> Result<()> {
    let destination = directory.join(&options.logo_path);

    let prompt = if destination.exists() {
        println!(
            "🖼️  Logo already present at {}",
            options.logo_path.display()
        );
        "Path to a replacement logo (Enter to keep the current one):"
    } else {
        "Path to a logo image (Enter to skip):"
    };

    let Some(source) = prompt_path(prompt, &["png", "jpg", "jpeg", "gif", "svg", "webp"])? else {
        return Ok(());
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &destination)?;
    println!("✅ Copied logo to {}", options.logo_path.display());

    Ok(())
}

fn prompt_record_template(directory: &Path, options: &ConfigurationOptions) -> Result<()> {
    const BUILTIN: &str = "Write the built-in template";
    const COPY: &str = "Copy an existing .typ file";
    const SKIP: &str = "Skip (the built-in template is used at runtime)";

    let destination = directory.join(&options.record_path);
    let exists = destination.exists();
    if exists {
        println!(
            "📄 Record template already present at {}",
            options.record_path.display()
        );
    }

    let choice = Select::new("Record template:", vec![BUILTIN, COPY, SKIP])
        // An existing template is left alone unless the user asks otherwise.
        .with_starting_cursor(if exists { 2 } else { 0 })
        .prompt()
        .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

    if choice == SKIP {
        return Ok(());
    }

    if exists
        && !confirm(
            &format!("Overwrite {}?", options.record_path.display()),
            false,
        )?
    {
        return Ok(());
    }

    let content = if choice == BUILTIN {
        BUILTIN_TEMPLATE.to_string()
    } else {
        let Some(source) = prompt_path("Path to a .typ template:", &["typ"])? else {
            return Ok(());
        };
        fs::read_to_string(&source)?
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&destination, content)?;
    println!("✅ Wrote {}", options.record_path.display());

    Ok(())
}

/// A checklist file on disk, identified by its title as the loader sees it.
struct ChecklistFile {
    title: String,
    path: PathBuf,
}

impl ChecklistFile {
    fn is_yaml(&self) -> bool {
        matches!(
            self.path
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref(),
            Some("yaml") | Some("yml")
        )
    }
}

/// List the checklist files the loader would pick up, sorted by title.
fn read_checklists(checklist_dir: &Path) -> Result<Vec<ChecklistFile>> {
    let mut checklists = Vec::new();

    for entry in fs::read_dir(checklist_dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }

        let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !CHECKLIST_EXTENSIONS.contains(&extension.to_lowercase().as_str()) {
            continue;
        }

        let title = match title_of(&path) {
            Some(title) => title,
            None => continue,
        };
        checklists.push(ChecklistFile { title, path });
    }

    checklists.sort_by(|a, b| a.title.cmp(&b.title));
    Ok(checklists)
}

/// The title the loader derives for a checklist file: the YAML root key, or the
/// file stem (backtick-unwrapped) for plain text checklists.
fn title_of(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    let is_yaml = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref(),
        Some("yaml") | Some("yml")
    );

    if is_yaml {
        return parse_yaml_checklist(&content).ok().map(|(title, _)| title);
    }

    let stem = path.file_stem().and_then(|s| s.to_str())?;
    Some(
        if stem.starts_with('`') && stem.ends_with('`') && stem.len() > 2 {
            stem[1..stem.len() - 1].to_string()
        } else {
            stem.to_string()
        },
    )
}

fn checklist_menu(checklist_dir: &Path) -> Result<()> {
    loop {
        let checklists = read_checklists(checklist_dir)?;

        let mut options = vec![DONE.to_string(), NEW_CHECKLIST.to_string()];
        options.extend(checklists.iter().map(|c| format!("📋 {}", c.title)));

        let selection = Select::new("Checklists:", options)
            .with_help_message(&format!(
                "{} checklist(s) in {}",
                checklists.len(),
                checklist_dir.display()
            ))
            .prompt()
            .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

        if selection == DONE {
            return Ok(());
        }

        if selection == NEW_CHECKLIST {
            new_checklist(checklist_dir, &checklists)?;
            continue;
        }

        let title = selection.strip_prefix("📋 ").unwrap_or(&selection);
        let Some(checklist) = checklists.iter().find(|c| c.title == title) else {
            continue;
        };
        edit_checklist(checklist, &checklists)?;
    }
}

fn new_checklist(checklist_dir: &Path, existing: &[ChecklistFile]) -> Result<()> {
    const EDITOR: &str = "Author it in an editor";
    const ITEMS: &str = "Add items one at a time";
    const STARTER: &str = "Start from a starter checklist";

    let Some(name) = prompt_checklist_name("Checklist name:", "", existing)? else {
        return Ok(());
    };

    let mode = Select::new(
        "How would you like to write it?",
        vec![EDITOR, ITEMS, STARTER],
    )
    .prompt()
    .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

    let content = match mode {
        ITEMS => build_checklist_interactively()?,
        STARTER => {
            let starter = Select::new(
                "Starter checklist:",
                STARTERS.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            )
            .prompt()
            .map_err(|e| anyhow!("Selection cancelled: {e}"))?;
            let body = STARTERS
                .iter()
                .find(|(n, _)| *n == starter)
                .map(|(_, body)| *body)
                .unwrap_or_default();
            match edit_markdown_checklist(&name, body)? {
                Some(content) => content,
                None => return Ok(()),
            }
        }
        _ => match edit_markdown_checklist(&name, MARKDOWN_SKELETON)? {
            Some(content) => content,
            None => return Ok(()),
        },
    };

    let path = checklist_dir.join(checklist_filename(&name, NEW_CHECKLIST_EXTENSION));
    if path.exists() && !confirm(&format!("Overwrite {}?", path.display()), false)? {
        return Ok(());
    }

    fs::write(&path, content)?;
    println!("✅ Wrote {}", path.display());

    Ok(())
}

/// Edit, rename, or delete an existing checklist.
fn edit_checklist(checklist: &ChecklistFile, existing: &[ChecklistFile]) -> Result<()> {
    const EDIT: &str = "Edit contents";
    const RENAME: &str = "Rename";
    const DELETE: &str = "Delete";

    let action = Select::new(
        &format!("{}:", checklist.title),
        vec![EDIT, RENAME, DELETE, BACK],
    )
    .with_help_message(&checklist.path.display().to_string())
    .prompt()
    .map_err(|e| anyhow!("Selection cancelled: {e}"))?;

    match action {
        EDIT => {
            // The raw file is edited, never the loaded checklist: a rendered
            // checklist would re-prepend the note and flatten YAML sections.
            let current = fs::read_to_string(&checklist.path)?;
            let edited = if checklist.is_yaml() {
                edit_yaml_checklist(&checklist.title, &current)?
            } else {
                edit_markdown_checklist(&checklist.title, &current)?
            };

            if let Some(content) = edited {
                fs::write(&checklist.path, content)?;
                println!("✅ Updated {}", checklist.path.display());
            }
        }
        RENAME => {
            let others: Vec<&ChecklistFile> = existing
                .iter()
                .filter(|c| c.path != checklist.path)
                .collect();
            let others: Vec<ChecklistFile> = others
                .into_iter()
                .map(|c| ChecklistFile {
                    title: c.title.clone(),
                    path: c.path.clone(),
                })
                .collect();

            let Some(new_name) = prompt_checklist_name("New name:", &checklist.title, &others)?
            else {
                return Ok(());
            };
            rename_checklist(checklist, &new_name)?;
        }
        DELETE if confirm(&format!("Delete {}?", checklist.path.display()), false)? => {
            fs::remove_file(&checklist.path)?;
            println!("🗑️  Deleted {}", checklist.path.display());
        }
        _ => {}
    }

    Ok(())
}

/// Rename a checklist: the YAML root key (or the backtick-wrapped file stem for
/// plain text checklists) is what the loader turns into the title.
fn rename_checklist(checklist: &ChecklistFile, new_name: &str) -> Result<()> {
    let directory = checklist
        .path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", checklist.path.display()))?;

    let new_path = if checklist.is_yaml() {
        let content = fs::read_to_string(&checklist.path)?;
        let retitled = retitle_yaml(&content, &checklist.title, new_name);
        fs::write(&checklist.path, retitled)?;

        let extension = checklist
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("yaml");
        // A YAML checklist takes its title from the root key, so the filename
        // only has to be stable and readable.
        directory.join(format!("{}.{extension}", slug(new_name)))
    } else {
        let extension = checklist
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or(NEW_CHECKLIST_EXTENSION);
        directory.join(checklist_filename(new_name, extension))
    };

    if new_path != checklist.path {
        if new_path.exists() && !confirm(&format!("Overwrite {}?", new_path.display()), false)? {
            return Ok(());
        }
        fs::rename(&checklist.path, &new_path)?;
    }

    println!(
        "✅ Renamed '{}' to '{new_name}' ({})",
        checklist.title,
        new_path.display()
    );

    Ok(())
}

/// Prompt for a checklist name that is non-empty and not already taken.
fn prompt_checklist_name(
    message: &str,
    default: &str,
    existing: &[ChecklistFile],
) -> Result<Option<String>> {
    let name = Text::new(message)
        .with_default(default)
        .prompt()
        .map_err(|e| anyhow!("Input cancelled: {e}"))?;

    let name = name.trim().to_string();
    if name.is_empty() {
        println!("⚠️  A checklist name is required; skipping");
        return Ok(None);
    }

    if existing.iter().any(|c| c.title == name) {
        println!("⚠️  A checklist named '{name}' already exists; skipping");
        return Ok(None);
    }

    Ok(Some(name))
}

/// Open `$EDITOR` on YAML checklist content, re-opening until it parses or the
/// user gives up.
fn edit_yaml_checklist(name: &str, initial: &str) -> Result<Option<String>> {
    let mut content = initial.to_string();

    loop {
        let Some(edited) = edit_raw(name, &content, ".yaml")? else {
            return Ok(None);
        };

        match parse_yaml_checklist(&edited) {
            Ok(_) => return Ok(Some(edited)),
            Err(e) => {
                println!("⚠️  {} is not a valid checklist: {e}", name.yellow());
                if !confirm("Edit again?", true)? {
                    return Ok(None);
                }
                content = edited;
            }
        }
    }
}

/// Open the editor on markdown checklist content, re-opening while it contains
/// no checklist items — a checklist without `- [ ]` lines tracks nothing.
fn edit_markdown_checklist(name: &str, initial: &str) -> Result<Option<String>> {
    let mut content = initial.to_string();

    loop {
        let Some(edited) = edit_raw(name, &content, ".md")? else {
            return Ok(None);
        };

        if edited.contains("- [ ]") {
            return Ok(Some(edited));
        }

        println!(
            "⚠️  {} has no checklist items; items are lines starting with '- [ ]'",
            name.yellow()
        );
        if !confirm("Edit again?", true)? {
            // Keeping it is legitimate — a checklist may be prose the QCer
            // reads rather than ticks.
            return Ok(if confirm("Save it anyway?", false)? {
                Some(edited)
            } else {
                None
            });
        }
        content = edited;
    }
}

fn edit_raw(name: &str, initial: &str, extension: &str) -> Result<Option<String>> {
    let edited = Editor::new(&format!("Edit '{name}':"))
        .with_predefined_text(initial)
        .with_file_extension(extension)
        .with_editor_command(&editor_command())
        .prompt()
        .map_err(|e| anyhow!("Editor cancelled: {e}"))?;

    if edited.trim().is_empty() {
        println!("⚠️  Empty content; discarding the edit");
        return Ok(None);
    }

    // The editor prompt strips the trailing newline; keep files POSIX-shaped.
    Ok(Some(if edited.ends_with('\n') {
        edited
    } else {
        format!("{edited}\n")
    }))
}

/// Build markdown checklist content from one-at-a-time prompts, optionally
/// grouped into sections.
fn build_checklist_interactively() -> Result<String> {
    let sectioned = confirm("Organize items into sections?", false)?;

    let mut markdown = String::new();

    if !sectioned {
        for item in prompt_items("Item (Enter to finish):")? {
            markdown.push_str(&format!("- [ ] {item}\n"));
        }
        return Ok(if markdown.is_empty() {
            MARKDOWN_SKELETON.to_string()
        } else {
            markdown
        });
    }

    loop {
        let section = Text::new("Section name (Enter to finish):")
            .prompt()
            .map_err(|e| anyhow!("Input cancelled: {e}"))?;
        let section = section.trim().to_string();
        if section.is_empty() {
            break;
        }

        let items = prompt_items(&format!("Item for '{section}' (Enter to finish):"))?;
        if items.is_empty() {
            println!("⚠️  '{section}' has no items; skipping the section");
            continue;
        }

        // Sections start at level 3, matching how YAML checklists render.
        markdown.push_str(&format!("### {section}\n\n"));
        for item in &items {
            markdown.push_str(&format!("- [ ] {item}\n"));
        }
        markdown.push('\n');
    }

    Ok(if markdown.is_empty() {
        MARKDOWN_SKELETON.to_string()
    } else {
        markdown.trim_end().to_string() + "\n"
    })
}

fn prompt_items(message: &str) -> Result<Vec<String>> {
    let mut items = Vec::new();

    loop {
        let item = Text::new(message)
            .prompt()
            .map_err(|e| anyhow!("Input cancelled: {e}"))?;
        let item = item.trim().to_string();
        if item.is_empty() {
            return Ok(items);
        }
        items.push(item);
    }
}

/// The editor to open checklists in.
///
/// `inquire` defaults to `nano`; `$VISUAL`/`$EDITOR` come first so the user's
/// configured editor wins, and `vim` is preferred over `nano` as the fallback.
fn editor_command() -> OsString {
    for var in ["VISUAL", "EDITOR"] {
        if let Some(editor) = std::env::var_os(var).filter(|e| !e.is_empty()) {
            return editor;
        }
    }

    for editor in ["vim", "vi", "nano"] {
        if which(editor) {
            return OsString::from(editor);
        }
    }

    OsString::from("vi")
}

/// Whether `command` is an executable on `PATH`.
fn which(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

/// Filename for a checklist whose title comes from the file stem (markdown and
/// plain text). Whitespace is preserved by backtick-wrapping the stem, which is
/// how `Configuration::load_checklists` reads titles containing spaces back.
fn checklist_filename(name: &str, extension: &str) -> String {
    // Path separators would silently redirect the write, so they are the one
    // thing that cannot survive into the filename.
    let sanitized = name.replace(['/', '\\'], "-");

    let stem = if sanitized.contains(char::is_whitespace) {
        format!("`{sanitized}`")
    } else {
        sanitized
    };

    format!("{stem}.{extension}")
}

/// Replace the YAML root key so a starter (or renamed) checklist carries the
/// chosen title. Comments and formatting elsewhere in the file are preserved.
fn retitle_yaml(content: &str, old_title: &str, new_title: &str) -> String {
    let old_key = yaml_key(old_title);
    let new_key = yaml_key(new_title);

    let mut out = Vec::new();
    let mut replaced = false;

    for line in content.lines() {
        if !replaced && line.starts_with(&format!("{old_key}:")) {
            out.push(line.replacen(&format!("{old_key}:"), &format!("{new_key}:"), 1));
            replaced = true;
        } else {
            out.push(line.to_string());
        }
    }

    let mut result = out.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Filename-safe stem for a checklist title. YAML checklists take their title
/// from the root key, so the filename only needs to be stable and readable.
fn slug(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator {
            slug.push('_');
            last_was_separator = true;
        }
    }

    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "checklist".to_string()
    } else {
        slug
    }
}

/// A YAML mapping key, quoted only when it needs to be.
fn yaml_key(name: &str) -> String {
    yaml_scalar(name).unwrap_or_else(|_| format!("{name:?}"))
}

/// Render a value as a YAML scalar using serde so quoting and escaping match
/// what the loader expects to read back.
fn yaml_scalar(value: &(impl Serialize + ?Sized)) -> Result<String> {
    let rendered = serde_yaml::to_string(value)?;
    Ok(rendered.trim_end().to_string())
}

/// Serialize `options.yaml` field by field so each option keeps an explanatory
/// comment, and unset optional fields stay commented out rather than written as
/// `null`.
fn options_yaml(options: &ConfigurationOptions) -> Result<String> {
    let mut out = String::from(
        "# ghqc configuration options\n\
         # Written by `ghqc configuration init`. See docs/configuration.md.\n\n",
    );

    let mut field = |comment: &str, key: &str, value: Option<String>| {
        out.push_str(&format!("# {comment}\n"));
        match value {
            Some(rendered) => out.push_str(&format!("{key}: {rendered}\n\n")),
            None => out.push_str(&format!("# {key}:\n\n")),
        }
    };

    field(
        "Note prepended to the top of every checklist",
        "prepended_checklist_note",
        options
            .prepended_checklist_note
            .as_ref()
            .map(yaml_scalar)
            .transpose()?,
    );
    field(
        "What to call checklists in the UI",
        "checklist_display_name",
        Some(yaml_scalar(&options.checklist_display_name)?),
    );
    field(
        "Whether collaborator metadata is detected and included",
        "include_collaborators",
        Some(yaml_scalar(&options.include_collaborators)?),
    );
    field(
        "Path to the logo within this repository",
        "logo_path",
        Some(yaml_scalar(&options.logo_path)?),
    );
    field(
        "Directory holding the checklists within this repository",
        "checklist_directory",
        Some(yaml_scalar(&options.checklist_directory)?),
    );
    field(
        "Path to the record template within this repository",
        "record_path",
        Some(yaml_scalar(&options.record_path)?),
    );
    field(
        "Web UI repository refresh rate in seconds (falls back to GHQC_UI_REFRESH_RATE, then 15)",
        "ui_repo_refresh_rate_seconds",
        options
            .ui_repo_refresh_rate_seconds
            .map(|s| yaml_scalar(&s))
            .transpose()?,
    );
    field(
        "Whether the Web UI may update this repository (falls back to GHQC_ALLOW_CONFIG_UPDATE, then true)",
        "allow_ui_config_update",
        options
            .allow_ui_config_update
            .map(|v| yaml_scalar(&v))
            .transpose()?,
    );

    Ok(out.trim_end().to_string() + "\n")
}

fn print_summary(directory: &Path, options: &ConfigurationOptions) -> Result<()> {
    println!("📁 {}", directory.display().bold());
    println!("   options.yaml");

    let checklist_dir = directory.join(&options.checklist_directory);
    let checklists = read_checklists(&checklist_dir).unwrap_or_default();
    println!(
        "   {}/ — {} checklist(s)",
        options.checklist_directory.display(),
        checklists.len()
    );
    for checklist in &checklists {
        println!("     - {}", checklist.title);
    }

    if directory.join(&options.logo_path).exists() {
        println!("   {}", options.logo_path.display());
    } else {
        println!(
            "   {} (missing — records will render without a logo)",
            options.logo_path.display().yellow()
        );
    }

    if directory.join(&options.record_path).exists() {
        println!("   {}", options.record_path.display());
    }

    println!(
        "\nNext: commit and push this directory, then point ghqc at it:\n  \
         cd {}\n  git init && git add . && git commit -m 'Initial configuration'\n  \
         export GHQC_CONFIG_REPO=<remote url>",
        directory.display()
    );

    Ok(())
}

fn confirm(message: &str, default: bool) -> Result<bool> {
    Confirm::new(message)
        .with_default(default)
        .prompt()
        .map_err(|e| anyhow!("Confirmation cancelled: {e}"))
}

fn text(message: &str, default: &str) -> Result<String> {
    let value = Text::new(message)
        .with_default(default)
        .prompt()
        .map_err(|e| anyhow!("Input cancelled: {e}"))?;

    let trimmed = value.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

/// Prompt for a value that may be cleared: an empty answer means "unset",
/// which is why it cannot reuse [`text`]'s default-on-empty behavior.
///
/// The current value is seeded into the input buffer rather than shown as a
/// default, so it can be edited or deleted and no `()` hint lingers on screen.
fn optional_text_with_help(
    message: &str,
    current: Option<&str>,
    help: &str,
) -> Result<Option<String>> {
    let mut prompt = Text::new(message);
    if let Some(current) = current.filter(|c| !c.is_empty()) {
        prompt = prompt.with_initial_value(current);
    }
    if !help.is_empty() {
        prompt = prompt.with_help_message(help);
    }

    let value = prompt
        .prompt()
        .map_err(|e| anyhow!("Input cancelled: {e}"))?;

    let trimmed = value.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

fn path_text(message: &str, default: &Path) -> Result<PathBuf> {
    let default = default.display().to_string();
    Ok(PathBuf::from(text(message, &default)?))
}

/// Prompt for a path to an existing file, browsing the filesystem with Tab.
///
/// Directories are suggested with a trailing `/` and `../` is always offered,
/// so the tree can be walked in both directions without retyping. `extensions`
/// filters which files are suggested; directories are always suggested so
/// navigation is never blocked. An empty answer returns `None`.
fn prompt_path(message: &str, extensions: &[&str]) -> Result<Option<PathBuf>> {
    let completer = PathCompleter {
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
    };

    loop {
        let input = Text::new(message)
            .with_autocomplete(completer.clone())
            .with_help_message("Tab to complete, directories end in /, empty to skip")
            .prompt()
            .map_err(|e| anyhow!("Input cancelled: {e}"))?;

        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }

        let path = expand_user(input);
        if path.is_file() {
            return Ok(Some(path));
        }

        // A mistyped path is far more likely than a deliberate one, so ask
        // again rather than silently moving on.
        println!("⚠️  {} is not a readable file", path.display().yellow());
    }
}

/// Filesystem completion for [`prompt_path`].
#[derive(Clone)]
struct PathCompleter {
    /// Lowercase extensions to suggest. Empty suggests every file.
    extensions: Vec<String>,
}

impl PathCompleter {
    /// Split typed input into the directory to list and the prefix to match,
    /// keeping the directory exactly as typed so suggestions stay in the same
    /// shape (relative stays relative, `~` stays `~`).
    fn split(input: &str) -> (String, &str) {
        match input.rfind('/') {
            Some(index) => (input[..=index].to_string(), &input[index + 1..]),
            None => (String::new(), input),
        }
    }

    fn matches_extension(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }

        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| self.extensions.contains(&e.to_lowercase()))
            .unwrap_or(false)
    }
}

impl Autocomplete for PathCompleter {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        let (typed_dir, prefix) = Self::split(input.trim());

        let directory = if typed_dir.is_empty() {
            PathBuf::from(".")
        } else {
            expand_user(&typed_dir)
        };

        let mut directories = Vec::new();
        let mut files = Vec::new();

        // Walking back up is as important as walking down.
        if "..".starts_with(prefix) {
            directories.push(format!("{typed_dir}../"));
        }

        let Ok(entries) = fs::read_dir(&directory) else {
            return Ok(directories);
        };

        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };

            // Hidden entries stay hidden until explicitly asked for.
            if name.starts_with('.') && !prefix.starts_with('.') {
                continue;
            }
            if !name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                continue;
            }

            let path = entry.path();
            if path.is_dir() {
                directories.push(format!("{typed_dir}{name}/"));
            } else if self.matches_extension(&path) {
                files.push(format!("{typed_dir}{name}"));
            }
        }

        directories.sort();
        files.sort();
        directories.extend(files);

        Ok(directories)
    }

    fn get_completion(
        &mut self,
        input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<inquire::autocompletion::Replacement, CustomUserError> {
        if let Some(suggestion) = highlighted_suggestion {
            return Ok(Some(suggestion));
        }

        // Nothing highlighted: complete the way a shell does, to the unique
        // match or to the longest prefix every match shares.
        let suggestions = self.get_suggestions(input)?;
        Ok(match suggestions.len() {
            0 => None,
            1 => Some(suggestions[0].clone()),
            _ => {
                let common = longest_common_prefix(&suggestions);
                (common.len() > input.trim().len()).then_some(common)
            }
        })
    }
}

/// The longest prefix shared by every value, on a character boundary.
fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix = String::new();
    for (index, c) in first.char_indices() {
        let candidate = &first[..index + c.len_utf8()];
        if values.iter().all(|value| value.starts_with(candidate)) {
            prefix = candidate.to_string();
        } else {
            break;
        }
    }

    prefix
}

/// Expand a leading `~` so pasted paths behave the way they do in a shell.
fn expand_user(path: &str) -> PathBuf {
    let Some(rest) = path.strip_prefix('~') else {
        return PathBuf::from(path);
    };

    let Some(home) = std::env::var_os("HOME") else {
        return PathBuf::from(path);
    };

    PathBuf::from(home).join(rest.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_filename_safe() {
        assert_eq!(slug("Code Review"), "code_review");
        assert_eq!(slug("R/Rmd Report!"), "r_rmd_report");
        assert_eq!(slug("  "), "checklist");
    }

    #[test]
    fn yaml_key_round_trips_names_needing_quotes() {
        assert_eq!(yaml_key("Code Review"), "Code Review");

        // A colon or a leading special character would otherwise break the
        // mapping; serde quotes exactly those.
        for name in ["Review: phase 1", "- dashed", "#hash", "true"] {
            let yaml = format!("{}:\n  - item\n", yaml_key(name));
            let (title, _) = parse_yaml_checklist(&yaml)
                .unwrap_or_else(|e| panic!("key {name:?} does not round trip: {e}"));
            assert_eq!(title, name);
        }
    }

    #[test]
    fn retitle_replaces_root_key_and_keeps_comments() {
        let content = "# a comment\nCode Review:\n  - item\n";
        let retitled = retitle_yaml(content, "Code Review", "New Name");
        assert_eq!(retitled, "# a comment\nNew Name:\n  - item\n");

        let (title, _) = parse_yaml_checklist(&retitled).unwrap();
        assert_eq!(title, "New Name");
    }

    #[test]
    fn starters_are_markdown_with_items() {
        for (name, body) in STARTERS {
            assert!(body.contains("- [ ]"), "starter {name} has no items");
            assert!(
                body.contains("### "),
                "starter {name} has no section headers"
            );
        }
    }

    #[test]
    fn skeleton_has_a_checklist_item() {
        assert!(MARKDOWN_SKELETON.contains("- [ ]"));
    }

    #[test]
    fn checklist_filename_round_trips_through_the_loader() {
        assert_eq!(checklist_filename("Report", "md"), "Report.md");
        // Spaces survive by backtick-wrapping, which is how the loader reads
        // titles containing spaces back out of the filename.
        assert_eq!(checklist_filename("Code Review", "md"), "`Code Review`.md");
        assert_eq!(checklist_filename("a/b", "md"), "a-b.md");

        let temp = tempfile::tempdir().unwrap();
        for name in ["Report", "Code Review"] {
            let path = temp.path().join(checklist_filename(name, "md"));
            fs::write(&path, "- [ ] item\n").unwrap();
            assert_eq!(title_of(&path).as_deref(), Some(name));
        }
    }

    #[test]
    fn markdown_checklists_are_loaded_by_the_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("checklists")).unwrap();
        fs::write(
            root.join("checklists/`Code Review`.md"),
            "### Correctness\n\n- [ ] does what it says\n- [ ] handles edge cases\n",
        )
        .unwrap();

        let mut configuration = crate::Configuration::from_path(root);
        configuration.load_checklists();

        let checklist = configuration
            .checklists
            .get("Code Review")
            .expect("markdown checklist was not loaded");
        assert_eq!(checklist.items(), 2);
    }

    #[test]
    fn path_completer_splits_on_the_last_separator() {
        assert_eq!(
            PathCompleter::split("src/cli/con"),
            ("src/cli/".into(), "con")
        );
        assert_eq!(PathCompleter::split("logo"), ("".into(), "logo"));
        assert_eq!(PathCompleter::split("../"), ("../".into(), ""));
    }

    #[test]
    fn path_completer_tab_completes_without_a_highlight() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(root.join("logo.png"), "x").unwrap();
        fs::write(root.join("logo-dark.png"), "x").unwrap();
        fs::write(root.join("banner.png"), "x").unwrap();

        let mut completer = PathCompleter {
            extensions: vec!["png".to_string()],
        };
        let dir = format!("{}/", root.display());

        // Several matches: complete as far as the prefix they share.
        let completion = completer.get_completion(&format!("{dir}lo"), None).unwrap();
        assert_eq!(completion, Some(format!("{dir}logo")));

        // Already at the shared prefix: nothing left to add.
        let completion = completer
            .get_completion(&format!("{dir}logo"), None)
            .unwrap();
        assert_eq!(completion, None);

        // A unique match completes all the way.
        let completion = completer
            .get_completion(&format!("{dir}ban"), None)
            .unwrap();
        assert_eq!(completion, Some(format!("{dir}banner.png")));

        // A highlighted suggestion always wins.
        let completion = completer
            .get_completion(&format!("{dir}l"), Some("chosen".to_string()))
            .unwrap();
        assert_eq!(completion, Some("chosen".to_string()));
    }

    #[test]
    fn edit_directory_prefers_the_flag_then_cwd_then_the_configured_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        let flagged = root.join("flagged");
        let cwd = root.join("cwd");
        let configured = root.join("configured");
        for dir in [&flagged, &cwd, &configured] {
            fs::create_dir(dir).unwrap();
            fs::write(dir.join("options.yaml"), "checklist_display_name: x\n").unwrap();
        }

        assert_eq!(
            resolve_edit_directory(Some(flagged.clone()), &cwd, &configured).unwrap(),
            flagged
        );
        assert_eq!(
            resolve_edit_directory(None, &cwd, &configured).unwrap(),
            cwd
        );

        // Only when the current directory is not itself a configuration
        // repository does the configured one win.
        fs::remove_file(cwd.join("options.yaml")).unwrap();
        assert_eq!(
            resolve_edit_directory(None, &cwd, &configured).unwrap(),
            configured
        );

        // Nothing to edit anywhere is an error, never a silent creation.
        fs::remove_file(configured.join("options.yaml")).unwrap();
        assert!(resolve_edit_directory(None, &cwd, &configured).is_err());
        assert!(resolve_edit_directory(Some(configured), &cwd, root).is_err());
    }

    #[test]
    fn edit_directory_resolves_a_relative_flag_against_the_working_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let repo = root.join("myconfig");
        fs::create_dir(&repo).unwrap();
        fs::write(repo.join("options.yaml"), "checklist_display_name: x\n").unwrap();

        assert_eq!(
            resolve_edit_directory(Some(PathBuf::from("myconfig")), root, root).unwrap(),
            repo
        );
    }

    #[test]
    fn longest_common_prefix_stops_at_the_first_difference() {
        let values = vec!["logo.png".to_string(), "logo-dark.png".to_string()];
        assert_eq!(longest_common_prefix(&values), "logo");
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn path_completer_suggests_directories_and_matching_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("assets")).unwrap();
        fs::write(root.join("logo.png"), "x").unwrap();
        fs::write(root.join("notes.txt"), "x").unwrap();
        fs::write(root.join(".hidden.png"), "x").unwrap();

        let mut completer = PathCompleter {
            extensions: vec!["png".to_string()],
        };

        let typed = format!("{}/", root.display());
        let suggestions = completer.get_suggestions(&typed).unwrap();
        let names: Vec<String> = suggestions
            .iter()
            .map(|s| s.trim_start_matches(&typed).to_string())
            .collect();

        // Directories first (so navigation always works), then matching files.
        assert_eq!(names, vec!["../", "assets/", "logo.png"]);
    }

    #[test]
    fn options_yaml_round_trips_and_comments_out_unset_options() {
        let options = ConfigurationOptions {
            prepended_checklist_note: Some("Note: edit items as needed".to_string()),
            checklist_display_name: "QC checklists".to_string(),
            ..Default::default()
        };

        let rendered = options_yaml(&options).unwrap();
        assert!(rendered.contains("# ui_repo_refresh_rate_seconds:"));
        assert!(rendered.contains("# allow_ui_config_update:"));

        let parsed: ConfigurationOptions = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(
            parsed.prepended_checklist_note,
            options.prepended_checklist_note
        );
        assert_eq!(
            parsed.checklist_display_name,
            options.checklist_display_name
        );
        assert_eq!(parsed.logo_path, options.logo_path);
        assert_eq!(parsed.record_path, options.record_path);
        assert_eq!(parsed.ui_repo_refresh_rate_seconds, None);
        assert_eq!(parsed.allow_ui_config_update, None);
    }

    #[test]
    fn options_yaml_writes_set_optional_values() {
        let options = ConfigurationOptions {
            ui_repo_refresh_rate_seconds: Some(30),
            allow_ui_config_update: Some(false),
            ..Default::default()
        };

        let rendered = options_yaml(&options).unwrap();
        let parsed: ConfigurationOptions = serde_yaml::from_str(&rendered).unwrap();
        assert_eq!(parsed.ui_repo_refresh_rate_seconds, Some(30));
        assert_eq!(parsed.allow_ui_config_update, Some(false));
    }

    #[test]
    fn read_checklists_reports_loader_visible_titles() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        fs::write(dir.join("code_review.yaml"), "Code Review:\n  - item\n").unwrap();
        fs::write(dir.join("`General Script`.txt"), "- [ ] item\n").unwrap();
        fs::write(dir.join("Report.md"), "- [ ] item\n").unwrap();
        // Not an extension the loader reads, so the wizard hides it too.
        fs::write(dir.join("README.rst"), "ignored\n").unwrap();

        let checklists = read_checklists(dir).unwrap();
        let titles: Vec<&str> = checklists.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["Code Review", "General Script", "Report"]);
    }
}
