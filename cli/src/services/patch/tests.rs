use super::{combine_patches, intersect_patches, parse_patch, ParsedPatch};

#[derive(Clone, Copy)]
struct PatchScenario {
    incremental: &'static [&'static str],
    post_commit: &'static str,
    golden: &'static str,
}

fn parse_fixtures(fixtures: &[&str]) -> Vec<ParsedPatch> {
    fixtures
        .iter()
        .map(|fixture| parse_patch(fixture).expect("fixture patch should parse"))
        .collect()
}

fn assert_reconstructs_post_commit(scenario: PatchScenario) {
    let combined = combine_patches(&parse_fixtures(scenario.incremental));
    let post_commit = parse_patch(scenario.post_commit).expect("fixture patch should parse");
    let golden: ParsedPatch =
        serde_json::from_str(scenario.golden).expect("golden json should load");

    assert_eq!(intersect_patches(&combined, &post_commit), golden);
}

#[test]
fn average_age_reconstruction_matches_post_commit() {
    assert_reconstructs_post_commit(PatchScenario {
        incremental: &[
            include_str!("fixtures/average_age_reconstruction/incremental_01.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_02.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_03.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_04.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_05.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_06.patch"),
            include_str!("fixtures/average_age_reconstruction/incremental_07.patch"),
        ],
        post_commit: include_str!("fixtures/average_age_reconstruction/post_commit.patch"),
        golden: include_str!("fixtures/average_age_reconstruction/golden.json"),
    });
}
