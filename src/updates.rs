use crate::api;
use eframe::egui;
use semver::Version;
use serde::Deserialize;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

#[derive(Default)]
pub(super) struct UpdateCheck {
    pub state: State,
    pub announce: bool,
}

#[derive(Default)]
pub(super) enum State {
    #[default]
    Idle,
    Checking { result: Receiver<Outcome>, announce: bool },
    Finished(Outcome),
}

pub(super) enum Outcome {
    Current,
    NoRelease,
    Unconfigured,
    Failed(CheckError),
    Available(Update),
}

pub(super) enum CheckError {
    Connection,
    RateLimited,
    Response,
}

pub(super) struct Update {
    pub version: Version,
    pub notes_url: String,
    pub download_url: Option<String>,
}

impl UpdateCheck {
    pub fn start(&mut self, ctx: &egui::Context, announce: bool) {
        if matches!(self.state, State::Checking { .. }) {
            return;
        }
        self.announce = false;
        let (send, result) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let installed = Version::parse(api::APP_VERSION).expect("Cargo package version");
            let _ = send.send(check_latest(REPOSITORY, &installed));
            ctx.request_repaint();
        });
        self.state = State::Checking { result, announce };
    }

    pub fn poll(&mut self) {
        let State::Checking { result, announce } = &self.state else { return };
        let outcome = match result.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Outcome::Failed(CheckError::Connection),
        };
        self.announce = *announce && matches!(outcome, Outcome::Available(_));
        self.state = State::Finished(outcome);
    }

    pub fn available(&self) -> Option<&Update> {
        match &self.state {
            State::Finished(Outcome::Available(update)) => Some(update),
            _ => None,
        }
    }
}

pub(super) fn check_latest(repository: &str, installed: &Version) -> Outcome {
    let Some(project) = repository.strip_prefix("https://github.com/") else {
        return Outcome::Unconfigured;
    };
    let Ok(client) = api::release_client() else {
        return Outcome::Failed(CheckError::Connection);
    };
    let Ok(response) = client.get(format!("https://api.github.com/repos/{project}/releases/latest"))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
    else {
        return Outcome::Failed(CheckError::Connection);
    };
    match response.status().as_u16() {
        200 => {},
        404 => return Outcome::NoRelease,
        403 | 429 => return Outcome::Failed(CheckError::RateLimited),
        _ => return Outcome::Failed(CheckError::Response),
    }
    let Ok(release) = response.json::<Release>() else {
        return Outcome::Failed(CheckError::Response);
    };
    release.compare(repository, installed)
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}

impl Release {
    fn compare(self, repository: &str, installed: &Version) -> Outcome {
        let tag = &self.tag_name;
        let Ok(version) = Version::parse(tag.strip_prefix('v').unwrap_or(tag)) else {
            return Outcome::Failed(CheckError::Response);
        };
        if self.draft || self.prerelease || !version.pre.is_empty() {
            return Outcome::NoRelease;
        }
        if !version.cmp_precedence(installed).is_gt() {
            return Outcome::Current;
        }
        let archive = format!("MediaLite-{version}-Windows-x64.zip");
        let download_url = self.assets.iter()
            .any(|asset| asset.state == "uploaded" && asset.name == archive && asset.size > 0)
            .then(|| format!("{repository}/releases/download/{tag}/{archive}"));
        Outcome::Available(Update {
            version,
            notes_url: format!("{repository}/releases/tag/{tag}"),
            download_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_only_the_uploaded_windows_archive_for_a_newer_stable_version() {
        let installed = Version::parse("0.1.9").unwrap();
        let release = serde_json::from_str::<Release>(r#"{
            "tag_name":"v0.1.10", "draft":false, "prerelease":false, "assets":[
                {"name":"MediaLite-0.1.10-Windows-x64.zip.sha256", "state":"uploaded", "size":100},
                {"name":"MediaLite-0.1.10-Windows-x64.zip", "state":"uploaded", "size":1000}
            ]
        }"#).unwrap();
        let Outcome::Available(update) = release.compare(REPOSITORY, &installed) else {
            panic!("expected a newer release");
        };
        assert_eq!(update.version.to_string(), "0.1.10");
        assert_eq!(update.download_url.as_deref(),
            Some(concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/download/v0.1.10/MediaLite-0.1.10-Windows-x64.zip")));
    }

    #[test]
    fn handles_current_prerelease_and_incomplete_uploads() {
        let installed = Version::parse("0.1.6").unwrap();
        let release = |tag: &str, prerelease| Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![Asset { name: "MediaLite-0.1.7-Windows-x64.zip".into(), state: "starter".into(), size: 0 }],
        };
        assert!(matches!(release("v0.1.6", false).compare(REPOSITORY, &installed), Outcome::Current));
        assert!(matches!(release("v0.1.7-rc.1", true).compare(REPOSITORY, &installed), Outcome::NoRelease));
        let Outcome::Available(update) = release("v0.1.7", false).compare(REPOSITORY, &installed) else {
            panic!("expected a newer release");
        };
        assert!(update.download_url.is_none());
        assert!(update.notes_url.ends_with("/releases/tag/v0.1.7"));
    }
}
