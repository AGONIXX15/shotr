use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Error, Result};

use crate::{
    config::{
        APP_ICON, MSG_COPIED, MSG_FAILED, MSG_PATH_COPIED, MSG_SAVED, NAMESPACE, PNG_MIME,
        STDIO_PATH, TEXT_MIME, ZENITY_COLOR_ARGS, ZENITY_SAVE_ARGS,
    },
    editor::{Editor, Request},
    output,
    settings::{Act, ExitTrigger, Settings, Thumbnail, exits_after},
    style::Color,
    util,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Done,
    Cancelled,
}

// where the overlay sits: above everything, or below the window of a dialog
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stacking {
    Overlay,
    BelowDialog,
}

// Runs the actions in order; true when the app should exit. A cancelled dialog ends the chain.
fn run_chain(
    acts: &[Act],
    early_exit: &[ExitTrigger],
    mut perform: impl FnMut(Act) -> Result<Outcome>,
) -> Result<bool> {
    for &act in acts {
        match perform(act)? {
            Outcome::Cancelled => return Ok(false),
            Outcome::Done if exits_after(early_exit, act) => return Ok(true),
            Outcome::Done => {}
        }
    }
    Ok(false)
}

// The things a request can do, borrowed from the overlay for the length of one call.
pub struct Actions<'a> {
    pub editor: &'a mut Editor,
    pub settings: &'a Settings,
    pub last_saved: &'a mut Option<PathBuf>,
    pub stack: &'a mut dyn FnMut(Stacking) -> Result<()>,
}

impl Actions<'_> {
    // true when the app should exit
    pub fn handle(&mut self, request: Request) -> bool {
        match request {
            Request::PickColor => {
                if let Err(e) = self.pick_color() {
                    self.report(&e);
                }
                false
            }
            Request::Run(acts) => self.run(&acts),
        }
    }

    pub fn run(&mut self, acts: &[Act]) -> bool {
        match run_chain(acts, &self.settings.early_exit, |act| self.perform(act)) {
            Ok(exit) => exit,
            Err(e) => {
                self.report(&e);
                false
            }
        }
    }

    pub fn auto_copy(&mut self) {
        if !self.settings.auto_copy || !self.editor.annotations_changed() {
            return;
        }
        if let Err(e) = self.copy_image(false) {
            self.report(&e);
        }
        // exporting commits an open draft, which is not a new change
        self.editor.annotations_changed();
    }

    pub fn perform(&mut self, act: Act) -> Result<Outcome> {
        match act {
            Act::Exit => {}
            Act::SaveToClipboard => {
                self.copy_image(true)?;
                if self.settings.save_after_copy {
                    self.save_file()?;
                }
            }
            Act::SaveToFile => {
                self.save_file()?;
            }
            Act::SaveToFileAs => return self.save_file_as(),
            Act::CopyFilepathToClipboard => {
                let path = match self.last_saved.clone() {
                    Some(path) => path,
                    None => self.save_file()?.context("nothing was saved to a file")?,
                };
                let text = path.to_string_lossy();
                let command = self.settings.copy_command.as_deref();
                output::copy(text.as_bytes(), TEXT_MIME, command)?;
                self.say(MSG_PATH_COPIED, Some(&path));
            }
        }
        Ok(Outcome::Done)
    }

    pub fn pick_color(&mut self) -> Result<()> {
        let program = self.settings.zenity.clone();
        let picked = self.with_dialog(|| util::zenity(&program, &ZENITY_COLOR_ARGS))?;
        if let Some(color) = picked.as_deref().and_then(Color::parse) {
            self.editor.set_color(color);
        }
        Ok(())
    }

    fn report(&self, error: &Error) {
        let message = format!("{error:#}");
        util::notify(self.settings.notifications, MSG_FAILED, &message, None);
    }

    // `shot` is the saved file or the thumbnail, used when the icon is the screenshot
    fn say(&self, message: &str, shot: Option<&Path>) {
        let icon = match self.settings.thumbnail {
            None => None,
            Some(Thumbnail::AppIcon) => Some(APP_ICON.to_owned()),
            Some(Thumbnail::Screenshot) => shot.map(|p| p.to_string_lossy().into_owned()),
        };
        let desktop = self.settings.notifications;
        util::notify(desktop, NAMESPACE, message, icon.as_deref());
    }

    fn copy_image(&mut self, announce: bool) -> Result<()> {
        let png = self.editor.export()?;
        output::copy(&png, PNG_MIME, self.settings.copy_command.as_deref())?;
        if announce {
            let wants_shot = self.settings.thumbnail == Some(Thumbnail::Screenshot);
            let shot = if wants_shot {
                Some(output::write_thumbnail(&png)?)
            } else {
                None
            };
            self.say(MSG_COPIED, shot.as_deref());
        }
        Ok(())
    }

    // None when the output is stdout
    fn save_file(&mut self) -> Result<Option<PathBuf>> {
        let png = self.editor.export()?;
        if self.settings.output_filename == STDIO_PATH {
            std::io::stdout()
                .write_all(&png)
                .context("could not write to stdout")?;
            return Ok(None);
        }
        let path = output::save(&png, &self.settings.output_filename)?;
        self.say(&format!("{MSG_SAVED} {}", path.display()), Some(&path));
        *self.last_saved = Some(path.clone());
        Ok(Some(path))
    }

    fn save_file_as(&mut self) -> Result<Outcome> {
        let png = self.editor.export()?;
        let suggested = output::suggested_path(&self.settings.output_filename)?;
        let filename = format!("--filename={}", suggested.display());
        let args = [&ZENITY_SAVE_ARGS[..], &[filename.as_str()]].concat();
        let program = self.settings.zenity.clone();
        let Some(chosen) = self.with_dialog(|| util::zenity(&program, &args))? else {
            return Ok(Outcome::Cancelled);
        };
        let path = PathBuf::from(chosen);
        output::save_to(&png, &path)?;
        self.say(&format!("{MSG_SAVED} {}", path.display()), Some(&path));
        *self.last_saved = Some(path);
        Ok(Outcome::Done)
    }

    // zenity is a normal window, so push the overlay below it meanwhile
    fn with_dialog<T>(&mut self, dialog: impl FnOnce() -> Result<T>) -> Result<T> {
        (self.stack)(Stacking::BelowDialog)?;
        let result = dialog();
        if let Err(e) = (self.stack)(Stacking::Overlay) {
            self.report(&e);
        }
        result
    }
}

#[cfg(test)]
// closures in place of method paths: the latter do not satisfy `with`'s higher-ranked bound
#[allow(clippy::redundant_closure_for_method_calls)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    use tiny_skia::Pixmap;

    use super::*;
    use crate::font::Fonts;

    const IMAGE: (u32, u32) = (40, 30);

    // a scratch directory with fake zenity and copy scripts
    struct Sandbox {
        dir: PathBuf,
        settings: Settings,
        editor: Editor,
        last_saved: Option<PathBuf>,
        scripts: u32,
    }

    impl Sandbox {
        // None when fontconfig has no font, so the tests skip
        fn new(name: &str) -> Option<Self> {
            let fonts = Fonts::load(None, None, &[]).ok()?;
            let dir = std::env::temp_dir().join(format!("shotr-{name}-{}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let mut settings = Settings::for_tests().unwrap();
            settings.notifications = false;
            settings.output_filename = dir.join("saved.png").to_string_lossy().into_owned();
            let base = Pixmap::new(IMAGE.0, IMAGE.1).unwrap();
            let editor = Editor::new(base, fonts, &settings, false).unwrap();
            Some(Self {
                dir,
                settings,
                editor,
                last_saved: None,
                scripts: 0,
            })
        }

        // a new file each time: rewriting a script another thread may be forking can hit ETXTBSY
        fn script(&mut self, body: &str) {
            self.scripts += 1;
            let path = self.dir.join(format!("fake-zenity-{}", self.scripts));
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            self.settings.zenity = path.to_string_lossy().into_owned();
        }

        // runs `f` with the stacking changes recorded
        fn with<R>(&mut self, f: impl FnOnce(&mut Actions) -> R) -> (R, Vec<Stacking>) {
            let mut changes = Vec::new();
            let mut stack = |s| {
                changes.push(s);
                Ok(())
            };
            let mut actions = Actions {
                editor: &mut self.editor,
                settings: &self.settings,
                last_saved: &mut self.last_saved,
                stack: &mut stack,
            };
            let result = f(&mut actions);
            (result, changes)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn save_as_writes_the_chosen_file_and_keeps_running() {
        let Some(mut sb) = Sandbox::new("saveas") else {
            return;
        };
        let target = sb.dir.join("chosen.png");
        sb.script(&format!("echo '{}'", target.display()));
        let (outcome, changes) = sb.with(|a| a.perform(Act::SaveToFileAs));
        assert_eq!(outcome.unwrap(), Outcome::Done);
        assert_eq!(changes, [Stacking::BelowDialog, Stacking::Overlay]);
        let saved = Pixmap::decode_png(&fs::read(&target).unwrap()).unwrap();
        assert_eq!((saved.width(), saved.height()), IMAGE);
        assert_eq!(sb.last_saved.as_deref(), Some(target.as_path()));
        let (exit, _) = sb.with(|a| a.run(&[Act::SaveToFileAs]));
        assert!(!exit, "no early exit configured");
        sb.settings.early_exit = vec![ExitTrigger::SaveAs];
        let (exit, _) = sb.with(|a| a.run(&[Act::SaveToFileAs]));
        assert!(exit);
    }

    #[test]
    fn cancelled_save_as_writes_nothing_and_stops_the_chain() {
        let Some(mut sb) = Sandbox::new("cancel") else {
            return;
        };
        sb.script("exit 1");
        let (outcome, changes) = sb.with(|a| a.perform(Act::SaveToFileAs));
        assert_eq!(outcome.unwrap(), Outcome::Cancelled);
        assert_eq!(changes, [Stacking::BelowDialog, Stacking::Overlay]);
        assert!(sb.last_saved.is_none());
        let (exit, _) = sb.with(|a| a.run(&[Act::SaveToFileAs, Act::Exit]));
        assert!(!exit, "the exit after a cancelled dialog must not run");
        let files = fs::read_dir(&sb.dir).unwrap().count();
        assert_eq!(files, 1, "only the fake script is in the directory");
    }

    #[test]
    fn zenity_failure_is_an_error_not_a_cancel() {
        let Some(mut sb) = Sandbox::new("fail") else {
            return;
        };
        sb.script("echo 'no display' >&2\nexit 5");
        let (outcome, changes) = sb.with(|a| a.perform(Act::SaveToFileAs));
        let message = format!("{:#}", outcome.unwrap_err());
        assert!(message.contains("no display"), "{message}");
        assert_eq!(
            changes,
            [Stacking::BelowDialog, Stacking::Overlay],
            "the overlay is restored"
        );
        let (picked, _) = sb.with(|a| a.pick_color());
        assert!(picked.is_err());
    }

    #[test]
    fn colour_picker_accepts_both_formats_and_keeps_the_colour_on_cancel() {
        let Some(mut sb) = Sandbox::new("colour") else {
            return;
        };
        assert_ne!(sb.editor.color(), Color::rgb(255, 0, 0));
        sb.script("echo 'rgb(10,200,30)'");
        sb.with(|a| a.pick_color()).0.unwrap();
        assert_eq!(sb.editor.color(), Color::rgb(10, 200, 30));
        sb.script("echo '#0ac81e'");
        sb.with(|a| a.pick_color()).0.unwrap();
        assert_eq!(sb.editor.color(), Color::rgb(10, 200, 30));
        sb.script("echo '#ff0000'");
        sb.with(|a| a.pick_color()).0.unwrap();
        assert_eq!(sb.editor.color(), Color::rgb(255, 0, 0));
        sb.script("exit 1");
        sb.with(|a| a.pick_color()).0.unwrap();
        assert_eq!(
            sb.editor.color(),
            Color::rgb(255, 0, 0),
            "cancel keeps the colour"
        );
    }

    #[test]
    fn copy_filepath_hands_the_saved_path_to_the_copy_command() {
        let Some(mut sb) = Sandbox::new("path") else {
            return;
        };
        let copied = sb.dir.join("copied.txt");
        sb.settings.copy_command = Some(format!("cat > '{}'", copied.display()));
        sb.with(|a| a.perform(Act::SaveToFile)).0.unwrap();
        sb.with(|a| a.perform(Act::CopyFilepathToClipboard))
            .0
            .unwrap();
        let saved = sb.last_saved.clone().unwrap();
        assert_eq!(
            fs::read_to_string(&copied).unwrap(),
            saved.to_string_lossy()
        );
    }

    #[test]
    fn copy_filepath_saves_first_when_nothing_was_saved() {
        let Some(mut sb) = Sandbox::new("path-first") else {
            return;
        };
        let copied = sb.dir.join("copied.txt");
        sb.settings.copy_command = Some(format!("cat > '{}'", copied.display()));
        sb.with(|a| a.perform(Act::CopyFilepathToClipboard))
            .0
            .unwrap();
        let saved = sb.last_saved.clone().unwrap();
        assert!(saved.exists());
        assert_eq!(
            fs::read_to_string(&copied).unwrap(),
            saved.to_string_lossy()
        );
    }

    #[test]
    fn chain_stops_at_a_cancelled_dialog() {
        let mut seen = Vec::new();
        let acts = [Act::SaveToClipboard, Act::SaveToFileAs, Act::Exit];
        let exit = run_chain(&acts, &[], |act| {
            seen.push(act);
            Ok(if act == Act::SaveToFileAs {
                Outcome::Cancelled
            } else {
                Outcome::Done
            })
        });
        assert!(!exit.unwrap());
        assert_eq!(seen, [Act::SaveToClipboard, Act::SaveToFileAs]);
    }

    #[test]
    fn chain_exits_on_exit_or_early_exit() {
        let done = |_| Ok(Outcome::Done);
        assert!(run_chain(&[Act::SaveToFile, Act::Exit], &[], done).unwrap());
        assert!(run_chain(&[Act::SaveToFile], &[ExitTrigger::Save], done).unwrap());
        assert!(!run_chain(&[Act::SaveToFile], &[ExitTrigger::Copy], done).unwrap());
    }
}
