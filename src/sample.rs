//! A small offline library, so the layout can be explored without a server.
//! All names are invented; nothing here is copied from anyone's catalogue.

use crate::model::*;

fn mi(
    id: &str,
    name: &str,
    kind: &str,
    year: i64,
    minutes: i64,
    rating: f64,
    genres: &[&str],
    overview: &str,
) -> MediaItem {
    MediaItem {
        id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
        overview: overview.to_string(),
        year: Some(year),
        runtime_ticks: Some(ticks_from_secs((minutes * 60) as f64)),
        community_rating: Some(rating),
        genres: genres.iter().map(|g| g.to_string()).collect(),
        official_rating: "PG-13".to_string(),
        ..Default::default()
    }
}

pub fn views() -> Vec<MediaItem> {
    vec![
        MediaItem {
            id: "lib-movies".into(),
            name: "Movies".into(),
            kind: "CollectionFolder".into(),
            ..Default::default()
        },
        MediaItem {
            id: "lib-shows".into(),
            name: "TV Shows".into(),
            kind: "CollectionFolder".into(),
            ..Default::default()
        },
        MediaItem {
            id: "lib-collections".into(),
            name: "Collections".into(),
            kind: "CollectionFolder".into(),
            ..Default::default()
        },
    ]
}

fn movies() -> Vec<MediaItem> {
    vec![
        mi(
            "sm1",
            "The Quiet Harbour",
            "Movie",
            2021,
            118,
            8.1,
            &["Drama", "Mystery"],
            "A lighthouse keeper finds a message in a bottle that should have been lost forty years ago, and sets out to deliver it.",
        ),
        mi(
            "sm2",
            "Neon District",
            "Movie",
            2023,
            134,
            7.6,
            &["Action", "Science Fiction"],
            "A courier in a rain-soaked megacity is hired to carry a package that half the city is willing to kill for.",
        ),
        mi(
            "sm3",
            "Small Hours",
            "Movie",
            2019,
            96,
            8.4,
            &["Drama"],
            "Two night-shift workers share a bus stop every morning and slowly tell each other the truth.",
        ),
        mi(
            "sm4",
            "Paper Constellations",
            "Movie",
            2022,
            107,
            7.9,
            &["Adventure", "Family"],
            "A girl maps the stars from her bedroom window and accidentally invents a new one.",
        ),
        mi(
            "sm5",
            "Northbound",
            "Movie",
            2018,
            121,
            7.2,
            &["Thriller"],
            "A night train, a missing passenger, and a conductor who has made this journey one time too many.",
        ),
        mi(
            "sm6",
            "Glass Orchard",
            "Movie",
            2020,
            112,
            8.0,
            &["Romance", "Drama"],
            "Two families share a greenhouse in a valley where the seasons have stopped keeping time.",
        ),
        mi(
            "sm7",
            "Signal Hill",
            "Movie",
            2024,
            128,
            7.4,
            &["Science Fiction", "Thriller"],
            "A radio astronomer hears a pattern in the noise that repeats exactly once every nine hours.",
        ),
        mi(
            "sm8",
            "The Long Way Home",
            "Movie",
            2017,
            104,
            8.2,
            &["Documentary"],
            "Four friends walk a coastline they last walked as children.",
        ),
        mi(
            "sm9",
            "Blue Hour",
            "Movie",
            2023,
            99,
            7.1,
            &["Mystery"],
            "A photographer develops a roll of film from a camera she does not remember owning.",
        ),
        mi(
            "sm10",
            "Every Third Sunday",
            "Movie",
            2016,
            93,
            7.8,
            &["Comedy", "Drama"],
            "A family's monthly lunch becomes the only place they can manage to argue honestly.",
        ),
    ]
}

fn series() -> Vec<MediaItem> {
    let mut out = Vec::new();
    for (n, (id, name, year, overview)) in [
        (
            "st1",
            "Coastal Lines",
            2022,
            "A small coastal town inherits a railway nobody remembers building.",
        ),
        (
            "st2",
            "The Archive",
            2021,
            "Librarians guard a collection that edits itself overnight.",
        ),
        (
            "st3",
            "Field Notes",
            2024,
            "A naturalist documents a forest that is quietly rearranging itself.",
        ),
        (
            "st4",
            "Second Shift",
            2020,
            "The night staff of a hospital keep the day staff alive.",
        ),
    ]
    .iter()
    .enumerate()
    {
        let mut it = mi(id, name, "Series", *year, 0, 8.3, &["Drama"], overview);
        it.child_count = Some(3 + (n as i64 % 2));
        it.runtime_ticks = None;
        out.push(it);
    }
    out
}

pub fn home() -> Vec<Row> {
    let all_movies = movies();
    let all_series = series();
    let resume = {
        let mut m1 = all_movies[0].clone();
        m1.progress_ticks = ticks_from_secs(34.0 * 60.0);
        let mut m2 = all_movies[3].clone();
        m2.progress_ticks = ticks_from_secs(12.0 * 60.0);
        vec![m1, m2]
    };
    let ep = fake_episode("st1", 1, 3, "The Third Rail", 47);
    let mut ep2 = fake_episode("st2", 1, 2, "Late Returns", 43);
    ep2.progress_ticks = ticks_from_secs(20.0 * 60.0);
    vec![
        Row {
            title: "Continue Watching".into(),
            items: resume,
        },
        Row {
            title: "Next Up".into(),
            items: vec![ep, ep2],
        },
        Row {
            title: "Recently Added".into(),
            items: all_movies.iter().take(8).cloned().collect(),
        },
        Row {
            title: "Movies".into(),
            items: all_movies.clone(),
        },
        Row {
            title: "TV Shows".into(),
            items: all_series.clone(),
        },
        Row {
            title: "Collections".into(),
            items: collections(),
        },
    ]
}

fn collections() -> Vec<MediaItem> {
    let mut a = MediaItem {
        id: "sb1".into(),
        name: "Quiet Evenings".into(),
        kind: "BoxSet".into(),
        overview: "Slow films for late nights.".into(),
        child_count: Some(4),
        ..Default::default()
    };
    a.genres = vec!["Drama".into()];
    let mut b = MediaItem {
        id: "sb2".into(),
        name: "Rain & Neon".into(),
        kind: "BoxSet".into(),
        overview: "Cities at night.".into(),
        child_count: Some(3),
        ..Default::default()
    };
    b.genres = vec!["Thriller".into()];
    vec![a, b]
}

fn fake_episode(series: &str, season: i64, number: i64, title: &str, minutes: i64) -> MediaItem {
    MediaItem {
        id: format!("{}e{}", series, season * 100 + number),
        name: title.to_string(),
        kind: "Episode".into(),
        series_id: Some(series.to_string()),
        season_id: Some(format!("{}s{}", series, season)),
        season_name: format!("Season {}", season),
        index: Some(number),
        parent_index: Some(season),
        overview: "The town wakes to find the tide has gone out further than anyone has seen."
            .to_string(),
        runtime_ticks: Some(ticks_from_secs((minutes * 60) as f64)),
        series_name: series_name(series),
        community_rating: Some(8.0),
        ..Default::default()
    }
}

fn series_name(id: &str) -> String {
    match id {
        "st1" => "Coastal Lines".to_string(),
        "st2" => "The Archive".to_string(),
        "st3" => "Field Notes".to_string(),
        _ => "Second Shift".to_string(),
    }
}

pub fn browse(parent: &str) -> (Vec<MediaItem>, i64) {
    let items = match parent {
        "lib-movies" => movies(),
        "lib-shows" => series(),
        "lib-collections" => collections(),
        _ => {
            let mut all = movies();
            all.extend(series());
            all
        }
    };
    let total = items.len() as i64;
    (items, total)
}

pub fn search(term: &str) -> Vec<MediaItem> {
    let needle = term.to_lowercase();
    let mut all = movies();
    all.extend(series());
    all.into_iter()
        .filter(|x| x.name.to_lowercase().contains(&needle))
        .collect()
}

/// What the sample user has starred, so the Favourites screen has something to show.
pub fn favorites() -> Vec<MediaItem> {
    let all_movies = movies();
    let mut picked: Vec<MediaItem> = vec![
        all_movies[2].clone(),
        all_movies[6].clone(),
        all_movies[8].clone(),
    ];
    for m in picked.iter_mut() {
        m.favorite = true;
    }
    picked
}

pub fn detail(id: &str) -> ItemDetail {
    let all_movies = movies();
    if let Some(m) = all_movies.iter().find(|x| x.id == id) {
        let mut similar: Vec<MediaItem> = all_movies
            .iter()
            .filter(|x| x.id != id)
            .take(8)
            .cloned()
            .collect();
        if similar.is_empty() {
            similar = all_movies.clone();
        }
        return ItemDetail {
            item: m.clone(),
            similar,
            ..Default::default()
        };
    }
    if let Some(c) = collections().into_iter().find(|x| x.id == id) {
        let items = movies();
        return ItemDetail {
            item: c,
            children: items.into_iter().take(4).collect(),
            ..Default::default()
        };
    }
    if let Some(s) = series().into_iter().find(|x| x.id == id) {
        let mut seasons = Vec::new();
        for n in 1..=3 {
            let mut season = MediaItem {
                id: format!("{}s{}", id, n),
                name: format!("Season {}", n),
                kind: "Season".into(),
                series_id: Some(id.to_string()),
                index: Some(n),
                ..Default::default()
            };
            season.child_count = Some(6);
            seasons.push(season);
        }
        let episodes: Vec<MediaItem> = (1..=6)
            .map(|n| {
                let titles = [
                    "The Tide Comes In",
                    "Late Returns",
                    "Salt and Paper",
                    "The Long Pier",
                    "Last Train",
                    "What the Water Kept",
                ];
                fake_episode(id, 1, n, titles[(n as usize - 1) % titles.len()], 44 + n)
            })
            .collect();
        return ItemDetail {
            item: s,
            seasons,
            episodes,
            ..Default::default()
        };
    }
    ItemDetail::default()
}

pub fn episodes(series: &str, season: i64) -> Vec<MediaItem> {
    let titles = [
        "The Tide Comes In",
        "Late Returns",
        "Salt and Paper",
        "The Long Pier",
        "Last Train",
        "What the Water Kept",
    ];
    (1..=6)
        .map(|n| {
            fake_episode(
                series,
                season,
                n,
                titles[(n as usize - 1) % titles.len()],
                42 + n * 2,
            )
        })
        .collect()
}
