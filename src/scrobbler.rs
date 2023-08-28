use std::{sync::Arc, time::{Duration, SystemTime, UNIX_EPOCH}};

use log::info;
use rustfm_scrobble::{Scrobble, Scrobbler};
use discord_presence::Client;

use crate::{config, library, spotify::PlayerEvent, model::{playable::Playable, track::Track}};

pub const DISCORD_APP_ID: u64 = 1145519858298138635;
pub const DISCORD_PLAYING: &str = "Playing";
pub const DISCORD_PAUSED: &str = "Paused";
pub const DISCORD_IMAGE_PLAY: &str = "playing";
pub const DISCORD_IMAGE_PAUSE: &str = "pause";
pub const DISCORD_IMAGE_LOGO: &str = "logo";

pub struct ScrobblerManager {
    // Optional scrobbler
    scrobbler: Option<Scrobbler>,
    // Discord Presence, required
    discord: Client,
    // Config
    cfg: Arc<config::Config>,
    library: Arc<library::Library>,
}

impl ScrobblerManager {
    pub fn new(cfg: Arc<config::Config>, library: Arc<library::Library>) -> ScrobblerManager {
        let drpc: Client = Client::new(DISCORD_APP_ID);

        let mut manager = ScrobblerManager {
            scrobbler: None,
            discord: drpc,
            cfg,
            library,
        };

        manager.set_lastfm();
        let _ = manager.discord.start();

        manager
    }

    pub fn set_lastfm(&mut self) {
        // Check if api_key and secret are set
        if let Some(scrobbling) = self.cfg.values().scrobbling.clone() {
            if let (Some(api_key), Some(api_secret)) = (scrobbling.lastfm_api_key.clone(), scrobbling.lastfm_api_secret.clone()) {
                let mut scrobbler = Scrobbler::new(&api_key, &api_secret);

                if let (Some(username), Some(password)) = (scrobbling.lastfm_username.clone(), scrobbling.lastfm_password.clone()) {
                    if let (Some(session_key), Some(session_user)) = (self.cfg.state().lastfm_session_key.clone(), self.cfg.state().lastfm_session_user.clone()) {
                        if session_user == username {
                            scrobbler.authenticate_with_session_key(&session_key);
                            info!("Authenticated with Last.fm using session key");
                            self.scrobbler = Some(scrobbler);
                            return;
                        }
                    }
                    let response = scrobbler.authenticate_with_password(&username, &password)
                        .expect("Failed to authenticate with Last.fm");
                    info!("Authenticated with Last.fm using username/password");

                    self.cfg.with_state_mut(|mut state| {
                        state.lastfm_session_key = Some(response.key.clone());
                        state.lastfm_session_user = Some(username.clone());
                    });
                    self.scrobbler = Some(scrobbler);
                }
            }
        }
    }

    fn playable_to_track(playable: Option<Playable>) -> Option<Track> {
        if let Some(playable) = playable {
            match playable {
                Playable::Track(track) => Some(track),
                _ => None
            }
        } else {
            None
        }
    }

    pub fn update_scrobbler(&mut self, state: PlayerEvent, playable: Option<Playable>, progress: Duration) {
        let is_enabled = self.cfg.values().scrobbling
            .clone()
            .unwrap_or(config::Scrobbling {
                enabled: Some(false),
                discord_enabled: Some(true),
                lastfm_api_key: None,
                lastfm_api_secret: None,
                lastfm_username: None,
                lastfm_password: None,
                discord_format_details: None,
                discord_format_state: None,
            })
            .enabled
            .clone()
            .unwrap_or(false);

        if !is_enabled {
            return;
        }

        if let (Some(track), Some(scrobbling_cfg)) = (Self::playable_to_track(playable), self.cfg.values().scrobbling.clone()) {
            let discord_details = Playable::format(
                &Playable::Track(track.clone()),
                &scrobbling_cfg.discord_format_details.clone().unwrap_or("%artists / %album".to_owned()),
                &self.library,
            );
            let discord_state = Playable::format(
                &Playable::Track(track.clone()),
                &scrobbling_cfg.discord_format_details.clone().unwrap_or("%title".to_owned()),
                &self.library,
            );
            let cover_url = track.cover_url
                .unwrap_or(String::from(DISCORD_IMAGE_LOGO));

            let discord_enabled = scrobbling_cfg.discord_enabled.clone().unwrap_or(true);

            match state {
                PlayerEvent::Playing(_) => {
                    if !discord_enabled {
                        return;
                    }

                    let unix_now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed_secs = progress.as_secs();
                    let start_timestamp = unix_now - elapsed_secs;
                    self.discord.set_activity(|act| {
                        act.details(discord_state)
                            .state(discord_details)
                            .assets(|assets| {
                                assets
                                    .large_image(cover_url)
                                    .large_text("ncspot")
                                    .small_image(DISCORD_IMAGE_PLAY)
                                    .small_text(DISCORD_PLAYING)
                            })
                            .timestamps(|ts| ts.start(start_timestamp))
                    }).expect("Failed to set Discord activity");
                }
                PlayerEvent::Stopped => {
                    self.discord.clear_activity()
                        .expect("Failed to clear Discord activity");
                }
                PlayerEvent::Paused(_) => {
                    if !discord_enabled {
                        return;
                    }

                    self.discord.clone().set_activity(|act| {
                        act.details(discord_state)
                            .state(discord_details)
                            .assets(|assets| {
                                assets
                                    .large_image(cover_url)
                                    .large_text("ncspot")
                                    .small_image(DISCORD_IMAGE_PAUSE)
                                    .small_text(DISCORD_PAUSED)
                            })
                    }).expect("Failed to set Discord activity");
                }
                PlayerEvent::FinishedTrack => {
                    if let Some(scrobbler) = &self.scrobbler {
                        let mut artists = track.artists.clone();
                        if artists.is_empty() {
                            artists.push(String::from("Unknown Artist"));
                        }
                        let scrub = Scrobble::new(
                            artists.join(", ").as_str(),
                            track.title.as_str(),
                            track.album.unwrap_or(String::from("Unknown Album")).as_str(),
                        );
    
                        info!("Scrobbling track: {} by {} ({})", scrub.track(), scrub.artist(), scrub.album());
                        let resp = scrobbler.scrobble(&scrub)
                            .expect("Failed to scrobble track");
                        info!("Scrobbled track: {}", resp.track);
                    }
                }
            }
        }
    }
}