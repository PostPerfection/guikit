use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{NaiveDateTime, TimeDelta};
use postkit::content_keys::ContentKeys;
use postkit::screening_playlist::{PlaylistRow, RowItem, ScreeningPlaylist};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use uuid::Uuid;

use super::PreviewPlayer;

const TICK_INTERVAL: Duration = Duration::from_millis(100);
const MILLISECONDS_PER_SECOND: f64 = 1000.0;

// the boundary the runner drives, the preview player in the app and a fake in the tests
pub trait RunnerPlayer {
    fn load(&self, source: &Path, keys: Option<ContentKeys>) -> Result<(), String>;
    fn queue_next(&self, source: &Path, keys: Option<ContentKeys>) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn status(&self) -> Result<PlayerStatus, String>;
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PlayerStatus {
    pub source: Option<String>,
    pub queued_source: Option<String>,
    pub eof: bool,
    pub position: Option<f64>,
    pub duration: Option<f64>,
}

// what a composition row plays, looked up by the app when the row loads so a KDM is picked then
pub struct RowSource {
    pub cpl_path: PathBuf,
    pub keys: Option<ContentKeys>,
}

pub type RowSourceLookup = Box<dyn Fn(&Path, Uuid) -> Result<RowSource, String> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    Playing,
    Holding,
    Finished,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerState {
    pub playlist_name: String,
    pub activity: Activity,
    pub current_row: Option<usize>,
    pub current_title: Option<String>,
    pub next_row: Option<usize>,
    pub next_title: Option<String>,
    // while holding, None is black
    pub still_image: Option<PathBuf>,
    pub seconds_to_next_start: Option<f64>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HoldKind {
    // black until the row's start time, then the row itself
    BeforeStart,
    // the intermission row itself, then the row after it
    Intermission,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Playing {
        row: usize,
        source: String,
        queued: Option<(usize, String)>,
        queue_attempted: bool,
    },
    Holding {
        row: usize,
        until: NaiveDateTime,
        kind: HoldKind,
    },
    Ended(Activity),
}

pub struct ScreeningRun {
    playlist: ScreeningPlaylist,
    lookup: RowSourceLookup,
    phase: Phase,
    errors: Vec<String>,
}

fn seconds_delta(seconds: f64) -> TimeDelta {
    TimeDelta::milliseconds((seconds * MILLISECONDS_PER_SECOND).round() as i64)
}

fn delta_seconds(delta: TimeDelta) -> f64 {
    delta.num_milliseconds() as f64 / MILLISECONDS_PER_SECOND
}

fn row_title(row: &PlaylistRow) -> String {
    match &row.item {
        RowItem::Composition { title, .. } => title.clone(),
        RowItem::Intermission { seconds, .. } => format!("Intermission, {seconds} s"),
    }
}

impl ScreeningRun {
    pub fn start(
        playlist: ScreeningPlaylist,
        first_row: usize,
        lookup: RowSourceLookup,
        player: &impl RunnerPlayer,
        now: NaiveDateTime,
    ) -> ScreeningRun {
        let mut run = ScreeningRun {
            playlist,
            lookup,
            phase: Phase::Ended(Activity::Stopped),
            errors: Vec::new(),
        };
        run.begin(first_row, true, player, now);
        run
    }

    pub fn is_running(&self) -> bool {
        !matches!(self.phase, Phase::Ended(_))
    }

    pub fn stop(&mut self) {
        if self.is_running() {
            eprintln!("[playlist] stopped");
        }
        self.phase = Phase::Ended(Activity::Stopped);
    }

    fn report_error(&mut self, error: String) {
        eprintln!("[playlist] {error}");
        self.errors.push(error);
    }

    // starts the row, or holds before it, skipping a composition that does not load
    fn begin(
        &mut self,
        first_row: usize,
        honour_start_time: bool,
        player: &impl RunnerPlayer,
        now: NaiveDateTime,
    ) {
        let mut honour_start_time = honour_start_time;
        for row in first_row..self.playlist.rows.len() {
            let entry = self.playlist.rows[row].clone();
            if let Some(start_time) = entry
                .start_time
                .filter(|start| honour_start_time && *start > now)
            {
                eprintln!(
                    "[playlist] holding black until {start_time} for row {}",
                    row + 1
                );
                self.hold(row, start_time, HoldKind::BeforeStart, player);
                return;
            }
            match &entry.item {
                RowItem::Intermission {
                    seconds,
                    still_image,
                } => {
                    let held = still_image
                        .as_ref()
                        .map_or("black".to_string(), |still| still.display().to_string());
                    eprintln!(
                        "[playlist] intermission row {}: holding {held} for {seconds} s",
                        row + 1
                    );
                    let until = now + seconds_delta(f64::from(*seconds));
                    self.hold(row, until, HoldKind::Intermission, player);
                    return;
                }
                RowItem::Composition {
                    package_directory,
                    cpl_id,
                    title,
                } => match self.load_row(package_directory, *cpl_id, player) {
                    Ok(source) => {
                        eprintln!("[playlist] row {} loaded: {title}", row + 1);
                        self.phase = Phase::Playing {
                            row,
                            source,
                            queued: None,
                            queue_attempted: false,
                        };
                        self.queue_following(player, now);
                        return;
                    }
                    Err(error) => {
                        self.report_error(format!("row {} ({title}) skipped: {error}", row + 1));
                        honour_start_time = true;
                    }
                },
            }
        }
        eprintln!("[playlist] finished");
        if let Err(error) = player.stop() {
            self.report_error(error);
        }
        self.phase = Phase::Ended(Activity::Finished);
    }

    fn hold(
        &mut self,
        row: usize,
        until: NaiveDateTime,
        kind: HoldKind,
        player: &impl RunnerPlayer,
    ) {
        if let Err(error) = player.stop() {
            self.report_error(error);
        }
        self.phase = Phase::Holding { row, until, kind };
    }

    fn load_row(
        &self,
        package_directory: &Path,
        cpl_id: Uuid,
        player: &impl RunnerPlayer,
    ) -> Result<String, String> {
        let source = (self.lookup)(package_directory, cpl_id)?;
        player.load(&source.cpl_path, source.keys)?;
        Ok(source.cpl_path.display().to_string())
    }

    pub fn tick(&mut self, player: &impl RunnerPlayer, now: NaiveDateTime) {
        match self.phase.clone() {
            Phase::Ended(_) => {}
            Phase::Holding { row, until, kind } => {
                if now < until {
                    return;
                }
                match kind {
                    HoldKind::BeforeStart => self.begin(row, false, player, now),
                    HoldKind::Intermission => self.begin(row + 1, true, player, now),
                }
            }
            Phase::Playing {
                row,
                source,
                queued,
                ..
            } => {
                let status = match player.status() {
                    Ok(status) => status,
                    Err(error) => {
                        self.report_error(error);
                        self.phase = Phase::Ended(Activity::Stopped);
                        return;
                    }
                };
                if let Some((queued_row, queued_source)) = queued {
                    // the queue empties at the hand-off, which tells the same composition twice in a row apart
                    let handed_off = status.queued_source.is_none()
                        && status.source.as_deref() == Some(queued_source.as_str());
                    if handed_off {
                        eprintln!(
                            "[playlist] row {} took over from row {} without a gap",
                            queued_row + 1,
                            row + 1
                        );
                        self.phase = Phase::Playing {
                            row: queued_row,
                            source: queued_source,
                            queued: None,
                            queue_attempted: false,
                        };
                        self.queue_following(player, now);
                        return;
                    }
                }
                if status.source.as_deref() != Some(source.as_str()) {
                    eprintln!(
                        "[playlist] the player left row {}, the playlist stops",
                        row + 1
                    );
                    self.phase = Phase::Ended(Activity::Stopped);
                    return;
                }
                if status.eof {
                    self.begin(row + 1, true, player, now);
                    return;
                }
                self.queue_following_after(&status, player, now);
            }
        }
    }

    fn queue_following(&mut self, player: &impl RunnerPlayer, now: NaiveDateTime) {
        match player.status() {
            Ok(status) => self.queue_following_after(&status, player, now),
            Err(error) => self.report_error(error),
        }
    }

    // a following composition with no start time, or one the current row ends after, plays on without a gap
    fn queue_following_after(
        &mut self,
        status: &PlayerStatus,
        player: &impl RunnerPlayer,
        now: NaiveDateTime,
    ) {
        let Phase::Playing {
            row,
            queued: None,
            queue_attempted: false,
            ..
        } = self.phase
        else {
            return;
        };
        let next = row + 1;
        let Some(entry) = self.playlist.rows.get(next).cloned() else {
            return;
        };
        let RowItem::Composition {
            package_directory,
            cpl_id,
            title,
        } = &entry.item
        else {
            return;
        };
        let remaining = match (status.duration, status.position) {
            (Some(duration), Some(position)) => (duration - position).max(0.0),
            _ => 0.0,
        };
        let current_end = now + seconds_delta(remaining);
        if entry.start_time.is_some_and(|start| start > current_end) {
            return;
        }
        let queued = (self.lookup)(package_directory, *cpl_id).and_then(|source| {
            player.queue_next(&source.cpl_path, source.keys)?;
            Ok(source.cpl_path.display().to_string())
        });
        let queued = match queued {
            Ok(queued_source) => {
                eprintln!(
                    "[playlist] row {} ({title}) queued after row {}",
                    next + 1,
                    row + 1
                );
                Some((next, queued_source))
            }
            Err(error) => {
                self.report_error(format!("row {} ({title}) not queued: {error}", next + 1));
                None
            }
        };
        if let Phase::Playing {
            queued: slot,
            queue_attempted,
            ..
        } = &mut self.phase
        {
            *slot = queued;
            *queue_attempted = true;
        }
    }

    pub fn state(&self, player: &impl RunnerPlayer, now: NaiveDateTime) -> RunnerState {
        let rows = &self.playlist.rows;
        let title_of = |row: usize| rows.get(row).map(row_title);
        let (activity, current_row, next_row, still_image, seconds_to_next_start) =
            match &self.phase {
                Phase::Ended(activity) => (*activity, None, None, None, None),
                Phase::Holding { row, until, kind } => {
                    let seconds_left = delta_seconds(*until - now).max(0.0);
                    let (current, next) = match kind {
                        HoldKind::BeforeStart => (None, Some(*row)),
                        HoldKind::Intermission => (Some(*row), Some(row + 1)),
                    };
                    let still = match (&rows[*row].item, kind) {
                        (RowItem::Intermission { still_image, .. }, HoldKind::Intermission) => {
                            still_image.clone()
                        }
                        _ => None,
                    };
                    (Activity::Holding, current, next, still, Some(seconds_left))
                }
                Phase::Playing { row, .. } => {
                    let remaining = player
                        .status()
                        .ok()
                        .and_then(|status| Some((status.duration? - status.position?).max(0.0)));
                    (
                        Activity::Playing,
                        Some(*row),
                        Some(row + 1),
                        None,
                        remaining,
                    )
                }
            };
        let next_row = next_row.filter(|row| *row < rows.len());
        RunnerState {
            playlist_name: self.playlist.name.clone(),
            activity,
            current_row,
            current_title: current_row.and_then(title_of),
            next_row,
            next_title: next_row.and_then(title_of),
            still_image,
            seconds_to_next_start,
            errors: self.errors.clone(),
        }
    }
}

struct PreviewRunnerPlayer<'a>(&'a PreviewPlayer);

impl RunnerPlayer for PreviewRunnerPlayer<'_> {
    fn load(&self, source: &Path, keys: Option<ContentKeys>) -> Result<(), String> {
        let player = self.0.player()?;
        self.0.forget_loaded_file();
        super::send_decode_scale_to_grok(player, self.0);
        player.load_source(&source.display().to_string(), keys)
    }

    fn queue_next(&self, source: &Path, keys: Option<ContentKeys>) -> Result<(), String> {
        self.0.player()?.grok().queue_next(source, keys)
    }

    fn stop(&self) -> Result<(), String> {
        let player = self.0.player()?;
        self.0.forget_loaded_file();
        player.stop()
    }

    fn status(&self) -> Result<PlayerStatus, String> {
        let metadata = self.0.player()?.grok().metadata_json();
        serde_json::from_str(&metadata).map_err(|error| format!("player metadata: {error}"))
    }
}

fn local_now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

// managed by an app that runs playlists, its thread ticks whatever run is current
pub struct ScreeningRunner {
    run: Arc<Mutex<Option<ScreeningRun>>>,
    app: tauri::AppHandle,
}

impl ScreeningRunner {
    pub fn new(app: tauri::AppHandle) -> ScreeningRunner {
        let run: Arc<Mutex<Option<ScreeningRun>>> = Arc::new(Mutex::new(None));
        let ticked = Arc::clone(&run);
        let ticking_app = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(TICK_INTERVAL);
            let mut current = ticked.lock().unwrap();
            let Some(run) = current.as_mut().filter(|run| run.is_running()) else {
                continue;
            };
            let state = ticking_app.state::<PreviewPlayer>();
            run.tick(&PreviewRunnerPlayer(&state), local_now());
        });
        ScreeningRunner { run, app }
    }

    pub fn start(
        &self,
        playlist: ScreeningPlaylist,
        first_row: usize,
        lookup: RowSourceLookup,
    ) -> RunnerState {
        let preview = self.app.state::<PreviewPlayer>();
        let player = PreviewRunnerPlayer(&preview);
        let now = local_now();
        // held across the start so the run it replaces cannot tick a load over it
        let mut current = self.run.lock().unwrap();
        eprintln!("[playlist] {} from row {}", playlist.name, first_row + 1);
        let run = ScreeningRun::start(playlist, first_row, lookup, &player, now);
        let state = run.state(&player, now);
        *current = Some(run);
        state
    }

    pub fn stop(&self) {
        if let Some(run) = self.run.lock().unwrap().as_mut() {
            run.stop();
        }
    }

    pub fn state(&self) -> Option<RunnerState> {
        let preview = self.app.state::<PreviewPlayer>();
        let current = self.run.lock().unwrap();
        current
            .as_ref()
            .map(|run| run.state(&PreviewRunnerPlayer(&preview), local_now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NOW: &str = "2026-10-06T19:00:00";
    const TRAILER: &str = "/library/trailer";
    const FEATURE: &str = "/library/feature";
    const SHORT: &str = "/library/short";
    const COMPOSITION_SECONDS: f64 = 60.0;

    #[derive(Default)]
    struct FakePlayer {
        status: RefCell<PlayerStatus>,
        calls: RefCell<Vec<String>>,
    }

    impl FakePlayer {
        fn calls(&self) -> Vec<String> {
            self.calls.take()
        }

        fn play_to(&self, position: f64) {
            self.status.borrow_mut().position = Some(position);
        }

        // what the grok player does at the last frame: hand off to the queued source or stop at the end
        fn reach_the_end(&self) {
            let mut status = self.status.borrow_mut();
            match status.queued_source.take() {
                Some(queued) => {
                    status.source = Some(queued);
                    status.position = Some(0.0);
                }
                None => status.eof = true,
            }
        }
    }

    impl RunnerPlayer for FakePlayer {
        fn load(&self, source: &Path, _keys: Option<ContentKeys>) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("load {}", source.display()));
            *self.status.borrow_mut() = PlayerStatus {
                source: Some(source.display().to_string()),
                queued_source: None,
                eof: false,
                position: Some(0.0),
                duration: Some(COMPOSITION_SECONDS),
            };
            Ok(())
        }

        fn queue_next(&self, source: &Path, _keys: Option<ContentKeys>) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("queue {}", source.display()));
            self.status.borrow_mut().queued_source = Some(source.display().to_string());
            Ok(())
        }

        fn stop(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("stop".to_string());
            *self.status.borrow_mut() = PlayerStatus::default();
            Ok(())
        }

        fn status(&self) -> Result<PlayerStatus, String> {
            Ok(self.status.borrow().clone())
        }
    }

    fn time(text: &str) -> NaiveDateTime {
        postkit::screening_playlist::parse_local_time(text).unwrap()
    }

    fn later(seconds: f64) -> NaiveDateTime {
        time(NOW) + seconds_delta(seconds)
    }

    fn composition(package: &str, start_time: Option<&str>) -> PlaylistRow {
        PlaylistRow {
            start_time: start_time.map(time),
            item: RowItem::Composition {
                package_directory: PathBuf::from(package),
                cpl_id: Uuid::nil(),
                title: package.rsplit('/').next().unwrap().to_string(),
            },
        }
    }

    fn intermission(seconds: u32) -> PlaylistRow {
        PlaylistRow {
            start_time: None,
            item: RowItem::Intermission {
                seconds,
                still_image: Some(PathBuf::from("/stills/interval.png")),
            },
        }
    }

    // the CPL sits in the package, and a package named missing has no KDM
    fn lookup() -> RowSourceLookup {
        Box::new(|package: &Path, _cpl_id: Uuid| {
            if package.ends_with("missing") {
                return Err("no KDM fits".to_string());
            }
            Ok(RowSource {
                cpl_path: package.join("CPL.xml"),
                keys: None,
            })
        })
    }

    fn start(rows: Vec<PlaylistRow>, player: &FakePlayer) -> ScreeningRun {
        let mut playlist = ScreeningPlaylist::new("Evening");
        playlist.rows = rows;
        ScreeningRun::start(playlist, 0, lookup(), player, time(NOW))
    }

    fn cpl(package: &str) -> String {
        format!("{package}/CPL.xml")
    }

    #[test]
    fn consecutive_compositions_are_queued_and_hand_off_without_a_gap() {
        let player = FakePlayer::default();
        let mut run = start(
            vec![
                composition(TRAILER, None),
                composition(FEATURE, None),
                composition(SHORT, None),
            ],
            &player,
        );
        assert_eq!(
            player.calls(),
            [
                format!("load {}", cpl(TRAILER)),
                format!("queue {}", cpl(FEATURE))
            ]
        );

        player.reach_the_end();
        run.tick(&player, later(60.0));

        assert_eq!(player.calls(), [format!("queue {}", cpl(SHORT))]);
        let state = run.state(&player, later(60.0));
        assert_eq!(state.activity, Activity::Playing);
        assert_eq!(state.current_row, Some(1));
        assert_eq!(state.current_title.as_deref(), Some("feature"));
        assert_eq!(state.next_title.as_deref(), Some("short"));
    }

    #[test]
    fn the_same_composition_twice_in_a_row_moves_on_only_at_the_hand_off() {
        let player = FakePlayer::default();
        let mut run = start(
            vec![composition(TRAILER, None), composition(TRAILER, None)],
            &player,
        );
        player.play_to(30.0);
        run.tick(&player, later(30.0));
        assert_eq!(run.state(&player, later(30.0)).current_row, Some(0));

        player.reach_the_end();
        run.tick(&player, later(60.0));

        assert_eq!(run.state(&player, later(60.0)).current_row, Some(1));
    }

    #[test]
    fn an_intermission_holds_its_still_for_its_length_then_the_next_row_loads() {
        let player = FakePlayer::default();
        let mut run = start(
            vec![
                composition(TRAILER, None),
                intermission(5),
                composition(FEATURE, None),
            ],
            &player,
        );
        assert_eq!(player.calls(), [format!("load {}", cpl(TRAILER))]);

        player.reach_the_end();
        run.tick(&player, later(60.0));
        assert_eq!(player.calls(), ["stop"]);
        let holding = run.state(&player, later(61.0));
        assert_eq!(holding.activity, Activity::Holding);
        assert_eq!(
            holding.still_image,
            Some(PathBuf::from("/stills/interval.png"))
        );
        assert_eq!(holding.seconds_to_next_start, Some(4.0));
        assert_eq!(holding.next_title.as_deref(), Some("feature"));

        run.tick(&player, later(64.9));
        assert_eq!(player.calls(), Vec::<String>::new());
        run.tick(&player, later(65.0));
        assert_eq!(player.calls(), [format!("load {}", cpl(FEATURE))]);
    }

    #[test]
    fn a_row_scheduled_after_the_row_before_ends_holds_black_until_its_start_time() {
        let player = FakePlayer::default();
        let mut run = start(
            vec![
                composition(TRAILER, None),
                composition(FEATURE, Some("2026-10-06T19:30:00")),
            ],
            &player,
        );
        assert_eq!(player.calls(), [format!("load {}", cpl(TRAILER))]);

        player.reach_the_end();
        run.tick(&player, later(60.0));
        assert_eq!(player.calls(), ["stop"]);
        let holding = run.state(&player, later(60.0));
        assert_eq!(holding.activity, Activity::Holding);
        assert_eq!(holding.still_image, None);
        assert_eq!(holding.seconds_to_next_start, Some(1740.0));

        run.tick(&player, time("2026-10-06T19:30:00"));
        assert_eq!(player.calls(), [format!("load {}", cpl(FEATURE))]);
    }

    #[test]
    fn a_row_scheduled_before_the_row_before_ends_is_queued_at_once() {
        let player = FakePlayer::default();
        start(
            vec![
                composition(TRAILER, None),
                composition(FEATURE, Some("2026-10-06T19:00:50")),
            ],
            &player,
        );

        assert_eq!(
            player.calls(),
            [
                format!("load {}", cpl(TRAILER)),
                format!("queue {}", cpl(FEATURE))
            ]
        );
    }

    #[test]
    fn a_pause_that_pushes_the_end_past_the_next_start_time_queues_the_next_row() {
        let player = FakePlayer::default();
        let mut run = start(
            vec![
                composition(TRAILER, None),
                composition(FEATURE, Some("2026-10-06T19:01:30")),
            ],
            &player,
        );
        assert_eq!(player.calls(), [format!("load {}", cpl(TRAILER))]);

        player.play_to(5.0);
        run.tick(&player, later(20.0));
        assert_eq!(player.calls(), Vec::<String>::new());

        run.tick(&player, later(40.0));
        assert_eq!(player.calls(), [format!("queue {}", cpl(FEATURE))]);
    }

    #[test]
    fn a_first_row_scheduled_later_holds_black_before_it() {
        let player = FakePlayer::default();
        let run = start(
            vec![composition(FEATURE, Some("2026-10-06T20:00:00"))],
            &player,
        );

        assert_eq!(player.calls(), ["stop"]);
        let holding = run.state(&player, time(NOW));
        assert_eq!(holding.activity, Activity::Holding);
        assert_eq!(holding.current_row, None);
        assert_eq!(holding.next_row, Some(0));
        assert_eq!(holding.seconds_to_next_start, Some(3600.0));
    }

    #[test]
    fn a_composition_that_does_not_load_is_skipped_and_reported() {
        let player = FakePlayer::default();
        let run = start(
            vec![
                composition("/library/missing", None),
                composition(FEATURE, None),
            ],
            &player,
        );

        assert_eq!(player.calls(), [format!("load {}", cpl(FEATURE))]);
        let state = run.state(&player, time(NOW));
        assert_eq!(state.current_row, Some(1));
        assert_eq!(state.errors, ["row 1 (missing) skipped: no KDM fits"]);
    }

    #[test]
    fn the_last_row_ending_finishes_the_playlist_and_stops_the_player() {
        let player = FakePlayer::default();
        let mut run = start(vec![composition(TRAILER, None)], &player);
        player.calls();

        player.reach_the_end();
        run.tick(&player, later(60.0));

        assert_eq!(player.calls(), ["stop"]);
        assert!(!run.is_running());
        assert_eq!(run.state(&player, later(60.0)).activity, Activity::Finished);
    }

    #[test]
    fn another_source_on_the_player_stops_the_playlist_and_leaves_the_player_alone() {
        let player = FakePlayer::default();
        let mut run = start(vec![composition(TRAILER, None), intermission(5)], &player);
        player.calls();

        player.load(Path::new("/elsewhere/CPL.xml"), None).unwrap();
        player.calls();
        run.tick(&player, later(1.0));

        assert_eq!(player.calls(), Vec::<String>::new());
        assert_eq!(run.state(&player, later(1.0)).activity, Activity::Stopped);
    }
}
