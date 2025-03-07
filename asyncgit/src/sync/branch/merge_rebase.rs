//! merging from upstream (rebase)

use crate::{
    error::{Error, Result},
    sync::utils,
};
use git2::BranchType;
use scopetime::scope_time;

/// trys merging current branch with its upstrema using rebase
pub fn merge_upstream_rebase(
    repo_path: &str,
    branch_name: &str,
) -> Result<()> {
    scope_time!("merge_upstream_rebase");

    let repo = utils::repo(repo_path)?;
    if super::get_branch_name_repo(&repo)? != branch_name {
        return Err(Error::Generic(String::from(
            "can only rebase in head branch",
        )));
    }

    let branch = repo.find_branch(branch_name, BranchType::Local)?;
    let upstream = branch.upstream()?;
    let upstream_commit = upstream.get().peel_to_commit()?;
    let annotated_upstream =
        repo.find_annotated_commit(upstream_commit.id())?;

    let mut rebase =
        repo.rebase(None, Some(&annotated_upstream), None, None)?;

    let signature =
        crate::sync::commit::signature_allow_undefined_name(&repo)?;

    while let Some(op) = rebase.next() {
        let _op = op?;
        // dbg!(op.id());

        if repo.index()?.has_conflicts() {
            rebase.abort()?;
            return Err(Error::Generic(String::from(
                "conflicts while merging",
            )));
        }

        rebase.commit(None, &signature, None)?;
    }

    rebase.finish(Some(&signature))?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::sync::{
        branch_compare_upstream, get_commits_info,
        remotes::{fetch_origin, push::push},
        tests::{
            debug_cmd_print, get_commit_ids, repo_clone,
            repo_init_bare, write_commit_file,
        },
        RepoState,
    };
    use git2::Repository;

    fn get_commit_msgs(r: &Repository) -> Vec<String> {
        let commits = get_commit_ids(r, 10);
        get_commits_info(
            r.workdir().unwrap().to_str().unwrap(),
            &commits,
            10,
        )
        .unwrap()
        .into_iter()
        .map(|c| c.message)
        .collect()
    }

    #[test]
    fn test_merge_normal() {
        let (r1_dir, _repo) = repo_init_bare().unwrap();

        let (clone1_dir, clone1) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone1_dir = clone1_dir.path().to_str().unwrap();

        // clone1

        let _commit1 =
            write_commit_file(&clone1, "test.txt", "test", "commit1");

        assert_eq!(clone1.head_detached().unwrap(), false);

        push(clone1_dir, "origin", "master", false, None, None)
            .unwrap();

        assert_eq!(clone1.head_detached().unwrap(), false);

        // clone2

        let (clone2_dir, clone2) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone2_dir = clone2_dir.path().to_str().unwrap();

        let _commit2 = write_commit_file(
            &clone2,
            "test2.txt",
            "test",
            "commit2",
        );

        assert_eq!(clone2.head_detached().unwrap(), false);

        push(clone2_dir, "origin", "master", false, None, None)
            .unwrap();

        assert_eq!(clone2.head_detached().unwrap(), false);

        // clone1

        let _commit3 = write_commit_file(
            &clone1,
            "test3.txt",
            "test",
            "commit3",
        );

        assert_eq!(clone1.head_detached().unwrap(), false);

        //lets fetch from origin
        let bytes =
            fetch_origin(clone1_dir, "master", None, None).unwrap();
        assert!(bytes > 0);

        //we should be one commit behind
        assert_eq!(
            branch_compare_upstream(clone1_dir, "master")
                .unwrap()
                .behind,
            1
        );

        // debug_cmd_print(clone1_dir, "git log");

        assert_eq!(clone1.head_detached().unwrap(), false);

        merge_upstream_rebase(clone1_dir, "master").unwrap();

        debug_cmd_print(clone1_dir, "git log");

        let state = crate::sync::repo_state(clone1_dir).unwrap();
        assert_eq!(state, RepoState::Clean);

        let commits = get_commit_msgs(&clone1);
        assert_eq!(
            commits,
            vec![
                String::from("commit3"),
                String::from("commit2"),
                String::from("commit1")
            ]
        );

        assert_eq!(clone1.head_detached().unwrap(), false);
    }

    #[test]
    fn test_merge_multiple() {
        let (r1_dir, _repo) = repo_init_bare().unwrap();

        let (clone1_dir, clone1) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone1_dir = clone1_dir.path().to_str().unwrap();

        // clone1

        write_commit_file(&clone1, "test.txt", "test", "commit1");

        push(clone1_dir, "origin", "master", false, None, None)
            .unwrap();

        // clone2

        let (clone2_dir, clone2) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone2_dir = clone2_dir.path().to_str().unwrap();

        write_commit_file(&clone2, "test2.txt", "test", "commit2");

        push(clone2_dir, "origin", "master", false, None, None)
            .unwrap();

        // clone1

        write_commit_file(&clone1, "test3.txt", "test", "commit3");
        write_commit_file(&clone1, "test4.txt", "test", "commit4");

        //lets fetch from origin

        fetch_origin(clone1_dir, "master", None, None).unwrap();

        merge_upstream_rebase(clone1_dir, "master").unwrap();

        debug_cmd_print(clone1_dir, "git log");

        let state = crate::sync::repo_state(clone1_dir).unwrap();
        assert_eq!(state, RepoState::Clean);

        let commits = get_commit_msgs(&clone1);
        assert_eq!(
            commits,
            vec![
                String::from("commit4"),
                String::from("commit3"),
                String::from("commit2"),
                String::from("commit1")
            ]
        );

        assert_eq!(clone1.head_detached().unwrap(), false);
    }

    #[test]
    fn test_merge_conflict() {
        let (r1_dir, _repo) = repo_init_bare().unwrap();

        let (clone1_dir, clone1) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone1_dir = clone1_dir.path().to_str().unwrap();

        // clone1

        let _commit1 =
            write_commit_file(&clone1, "test.txt", "test", "commit1");

        push(clone1_dir, "origin", "master", false, None, None)
            .unwrap();

        // clone2

        let (clone2_dir, clone2) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone2_dir = clone2_dir.path().to_str().unwrap();

        let _commit2 = write_commit_file(
            &clone2,
            "test2.txt",
            "test",
            "commit2",
        );

        push(clone2_dir, "origin", "master", false, None, None)
            .unwrap();

        // clone1

        let _commit3 =
            write_commit_file(&clone1, "test2.txt", "foo", "commit3");

        let bytes =
            fetch_origin(clone1_dir, "master", None, None).unwrap();
        assert!(bytes > 0);

        assert_eq!(
            branch_compare_upstream(clone1_dir, "master")
                .unwrap()
                .behind,
            1
        );

        let res = merge_upstream_rebase(clone1_dir, "master");
        assert!(res.is_err());

        let state = crate::sync::repo_state(clone1_dir).unwrap();

        assert_eq!(state, RepoState::Clean);

        let commits = get_commit_msgs(&clone1);
        assert_eq!(
            commits,
            vec![String::from("commit3"), String::from("commit1")]
        );
    }
}
