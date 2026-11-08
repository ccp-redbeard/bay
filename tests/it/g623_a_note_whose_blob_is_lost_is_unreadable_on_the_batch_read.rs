//! G623 (BACKLOG 174). A note whose blob the store lacks is unreadable on the batch read of the
//! evidence notes too, never "no records": `git cat-file --batch` answers `missing` for it as it
//! does for a tree with no note, and `git notes show` says `bad object` (G104b). At every fanout
//! the notes tree holds the note, the batch read names the producer, the tree and the blob, and
//! once the blob is back it reads the record again.

use std::path::Path;

use bay::evidence;
use bay::git::Git;

use crate::g196_evidence_reads_spawn_a_constant_number_of_git_processes::{fanned_note, landed};
use crate::support::TestBox;

/// Removes `name`'s note blob on `tree` from the store, and returns its id.
fn lose_blob(test_box: &TestBox, repo: &Path, tree: &str, name: &str, levels: usize) -> String {
    let mut path = String::new();
    for level in 0..levels {
        path.push_str(&tree[level * 2..level * 2 + 2]);
        path.push('/');
    }
    path.push_str(&tree[levels * 2..]);
    let blob = test_box.git_ok(
        repo,
        &[
            "rev-parse",
            &format!("{}:{path}", evidence::notes_ref(name)),
        ],
    );
    let objects = test_box.git_ok(repo, &["rev-parse", "--git-path", "objects"]);
    let loose = repo.join(objects).join(&blob[..2]).join(&blob[2..]);
    std::fs::remove_file(&loose).unwrap_or_else(|error| panic!("{}: {error}", loose.display()));
    blob
}

#[test]
fn a_note_whose_blob_is_lost_is_unreadable_at_every_fanout_and_reads_again_once_it_is_back() {
    let (test_box, repo, _, tree) = landed();
    let git = Git::new(&repo);
    let before = evidence::records_for(&git, &tree).unwrap();
    for levels in [0, 1] {
        let name = format!("lost-{levels}");
        fanned_note(&test_box, &repo, &tree, &name, levels);
        let blob = lose_blob(&test_box, &repo, &tree, &name, levels);

        let shown = evidence::producer_records(&git, &name, &tree)
            .expect_err("notes show of a lost blob is unreadable (G104b)");
        assert!(shown.contains("bad object"), "{shown}");
        let read = evidence::records_for(&git, &tree)
            .expect_err("the batch read of a lost blob is unreadable, never no records");
        assert!(
            read.contains(&evidence::notes_ref(&name)) && read.contains(&tree),
            "the error names the producer's notes and the tree: {read}"
        );
        assert!(read.contains(&format!("bad object {blob}")), "{read}");
        let many = evidence::records_for_many(&git, &[tree.as_str(), tree.as_str()])
            .expect_err("a read of several trees is unreadable too");
        assert!(many.contains(&blob), "{many}");

        // The same note text hashes to the same blob: back in the store, the record reads.
        let text = test_box.path().join(format!("{name}.json"));
        test_box.git_ok(&repo, &["hash-object", "-w", text.to_str().unwrap()]);
        let back = evidence::records_for(&git, &tree).unwrap();
        assert!(
            back.iter().any(|record| record.producer.name == name),
            "{back:?}"
        );
    }
    assert_eq!(
        evidence::records_for(&git, &tree).unwrap().len(),
        before.len() + 2
    );
}

#[test]
fn a_tree_with_no_note_still_reads_no_records() {
    let (test_box, repo, _, tree) = landed();
    let git = Git::new(&repo);
    fanned_note(&test_box, &repo, &tree, "elsewhere", 1);
    // The empty tree: the producer's notes tree holds a note, but not on it.
    let empty = test_box.git_ok(&repo, &["hash-object", "-t", "tree", "/dev/null"]);
    assert_ne!(empty, tree);
    let records = evidence::records_for(&git, &empty).expect("no note is no records");
    assert!(
        records
            .iter()
            .all(|record| record.producer.name != "elsewhere"),
        "{records:?}"
    );
}
