//! branch functions

pub mod merge_commit;
pub mod merge_ff;
pub mod merge_rebase;
pub mod rename;

use super::{
    remotes::get_default_remote_in_repo, utils::bytes2string,
};
use crate::{
    error::{Error, Result},
    sync::{utils, CommitId},
};
use git2::{BranchType, Repository};
use scopetime::scope_time;
use utils::get_head_repo;

/// returns the branch-name head is currently pointing to
/// this might be expensive, see `cached::BranchName`
pub(crate) fn get_branch_name(repo_path: &str) -> Result<String> {
    let repo = utils::repo(repo_path)?;

    get_branch_name_repo(&repo)
}

/// ditto
pub(crate) fn get_branch_name_repo(
    repo: &Repository,
) -> Result<String> {
    scope_time!("get_branch_name_repo");

    let iter = repo.branches(None)?;

    for b in iter {
        let b = b?;

        if b.0.is_head() {
            let name = b.0.name()?.unwrap_or("");
            return Ok(name.into());
        }
    }

    Err(Error::NoHead)
}

///
#[derive(Debug)]
pub struct LocalBranch {
    ///
    pub is_head: bool,
    ///
    pub has_upstream: bool,
    ///
    pub remote: Option<String>,
}

///
#[derive(Debug)]
pub enum BranchDetails {
    ///
    Local(LocalBranch),
    ///
    Remote,
}

///
#[derive(Debug)]
pub struct BranchInfo {
    ///
    pub name: String,
    ///
    pub reference: String,
    ///
    pub top_commit_message: String,
    ///
    pub top_commit: CommitId,
    ///
    pub details: BranchDetails,
}

impl BranchInfo {
    /// returns details about local branch or None
    pub fn local_details(&self) -> Option<&LocalBranch> {
        if let BranchDetails::Local(details) = &self.details {
            return Some(details);
        }

        None
    }
}

/// returns a list of `BranchInfo` with a simple summary on each branch
/// `local` filters for local branches otherwise remote branches will be returned
pub fn get_branches_info(
    repo_path: &str,
    local: bool,
) -> Result<Vec<BranchInfo>> {
    scope_time!("get_branches_info");

    let filter = if local {
        BranchType::Local
    } else {
        BranchType::Remote
    };

    let repo = utils::repo(repo_path)?;
    let mut branches_for_display: Vec<BranchInfo> = repo
        .branches(Some(filter))?
        .map(|b| {
            let branch = b?.0;
            let top_commit = branch.get().peel_to_commit()?;
            let reference = bytes2string(branch.get().name_bytes())?;
            let upstream = branch.upstream();

            let remote = repo
                .branch_upstream_remote(&reference)
                .ok()
                .as_ref()
                .and_then(|buf| buf.as_str())
                .map(String::from);

            let details = if local {
                BranchDetails::Local(LocalBranch {
                    is_head: branch.is_head(),
                    has_upstream: upstream.is_ok(),
                    remote,
                })
            } else {
                BranchDetails::Remote
            };

            Ok(BranchInfo {
                name: bytes2string(branch.name_bytes()?)?,
                reference,
                top_commit_message: bytes2string(
                    top_commit.summary_bytes().unwrap_or_default(),
                )?,
                top_commit: top_commit.id().into(),
                details,
            })
        })
        .filter_map(Result::ok)
        .collect();

    branches_for_display.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(branches_for_display)
}

///
#[derive(Debug, Default)]
pub struct BranchCompare {
    ///
    pub ahead: usize,
    ///
    pub behind: usize,
}

///
pub(crate) fn branch_set_upstream(
    repo: &Repository,
    branch_name: &str,
) -> Result<()> {
    scope_time!("branch_set_upstream");

    let mut branch =
        repo.find_branch(branch_name, BranchType::Local)?;

    if branch.upstream().is_err() {
        let remote = get_default_remote_in_repo(repo)?;
        let upstream_name = format!("{}/{}", remote, branch_name);
        branch.set_upstream(Some(upstream_name.as_str()))?;
    }

    Ok(())
}

/// returns remote of the upstream tracking branch for `branch`
pub fn get_branch_remote(
    repo_path: &str,
    branch: &str,
) -> Result<Option<String>> {
    let repo = utils::repo(repo_path)?;
    let branch = repo.find_branch(branch, BranchType::Local)?;
    let reference = bytes2string(branch.get().name_bytes())?;
    let remote_name = repo.branch_upstream_remote(&reference).ok();
    if let Some(remote_name) = remote_name {
        Ok(Some(bytes2string(remote_name.as_ref())?))
    } else {
        Ok(None)
    }
}

/// returns whether the pull merge strategy is set to rebase
pub fn config_is_pull_rebase(repo_path: &str) -> Result<bool> {
    let repo = utils::repo(repo_path)?;
    let config = repo.config()?;

    if let Ok(rebase) = config.get_entry("pull.rebase") {
        let value =
            rebase.value().map(String::from).unwrap_or_default();
        return Ok(value == "true");
    };

    Ok(false)
}

///
pub fn branch_compare_upstream(
    repo_path: &str,
    branch: &str,
) -> Result<BranchCompare> {
    scope_time!("branch_compare_upstream");

    let repo = utils::repo(repo_path)?;

    let branch = repo.find_branch(branch, BranchType::Local)?;

    let upstream = branch.upstream()?;

    let branch_commit =
        branch.into_reference().peel_to_commit()?.id();

    let upstream_commit =
        upstream.into_reference().peel_to_commit()?.id();

    let (ahead, behind) =
        repo.graph_ahead_behind(branch_commit, upstream_commit)?;

    Ok(BranchCompare { ahead, behind })
}

/// Modify HEAD to point to a branch then checkout head, does not work if there are uncommitted changes
pub fn checkout_branch(
    repo_path: &str,
    branch_ref: &str,
) -> Result<()> {
    scope_time!("checkout_branch");

    // This defaults to a safe checkout, so don't delete anything that
    // hasn't been committed or stashed, in this case it will Err
    let repo = utils::repo(repo_path)?;
    let cur_ref = repo.head()?;
    let statuses = repo.statuses(Some(
        git2::StatusOptions::new().include_ignored(false),
    ))?;

    if statuses.is_empty() {
        repo.set_head(branch_ref)?;

        if let Err(e) = repo.checkout_head(Some(
            git2::build::CheckoutBuilder::new().force(),
        )) {
            // This is safe beacuse cur_ref was just found
            repo.set_head(
                bytes2string(cur_ref.name_bytes())?.as_str(),
            )?;
            return Err(Error::Git(e));
        }
        Ok(())
    } else {
        Err(Error::UncommittedChanges)
    }
}

///
pub fn checkout_remote_branch(
    repo_path: &str,
    branch: &BranchInfo,
) -> Result<()> {
    scope_time!("checkout_remote_branch");

    let repo = utils::repo(repo_path)?;
    let cur_ref = repo.head()?;

    if !repo
        .statuses(Some(
            git2::StatusOptions::new().include_ignored(false),
        ))?
        .is_empty()
    {
        return Err(Error::UncommittedChanges);
    }

    let name = if let Some(pos) = branch.name.rfind('/') {
        branch.name[pos..].to_string()
    } else {
        branch.name.clone()
    };

    let commit = repo.find_commit(branch.top_commit.into())?;
    let mut new_branch = repo.branch(&name, &commit, false)?;
    new_branch.set_upstream(Some(&branch.name))?;

    repo.set_head(
        bytes2string(new_branch.into_reference().name_bytes())?
            .as_str(),
    )?;

    if let Err(e) = repo.checkout_head(Some(
        git2::build::CheckoutBuilder::new().force(),
    )) {
        // This is safe beacuse cur_ref was just found
        repo.set_head(bytes2string(cur_ref.name_bytes())?.as_str())?;
        return Err(Error::Git(e));
    }
    Ok(())
}

/// The user must not be on the branch for the branch to be deleted
pub fn delete_branch(
    repo_path: &str,
    branch_ref: &str,
) -> Result<()> {
    scope_time!("delete_branch");

    let repo = utils::repo(repo_path)?;
    let branch_as_ref = repo.find_reference(branch_ref)?;
    let mut branch = git2::Branch::wrap(branch_as_ref);
    if !branch.is_head() {
        branch.delete()?;
    } else {
        return Err(Error::Generic("You cannot be on the branch you want to delete, switch branch, then delete this branch".to_string()));
    }
    Ok(())
}

/// creates a new branch pointing to current HEAD commit and updating HEAD to new branch
pub fn create_branch(repo_path: &str, name: &str) -> Result<()> {
    scope_time!("create_branch");

    let repo = utils::repo(repo_path)?;

    let head_id = get_head_repo(&repo)?;
    let head_commit = repo.find_commit(head_id.into())?;

    let branch = repo.branch(name, &head_commit, false)?;
    let branch_ref = branch.into_reference();
    let branch_ref_name = bytes2string(branch_ref.name_bytes())?;
    repo.set_head(branch_ref_name.as_str())?;

    Ok(())
}

#[cfg(test)]
mod tests_branch_name {
    use super::*;
    use crate::sync::tests::{repo_init, repo_init_empty};

    #[test]
    fn test_smoke() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert_eq!(
            get_branch_name(repo_path).unwrap().as_str(),
            "master"
        );
    }

    #[test]
    fn test_empty_repo() {
        let (_td, repo) = repo_init_empty().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert!(matches!(
            get_branch_name(repo_path),
            Err(Error::NoHead)
        ));
    }
}

#[cfg(test)]
mod tests_create_branch {
    use super::*;
    use crate::sync::tests::repo_init;

    #[test]
    fn test_smoke() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        create_branch(repo_path, "branch1").unwrap();

        assert_eq!(
            get_branch_name(repo_path).unwrap().as_str(),
            "branch1"
        );
    }
}

#[cfg(test)]
mod tests_branch_compare {
    use super::*;
    use crate::sync::tests::repo_init;

    #[test]
    fn test_smoke() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        create_branch(repo_path, "test").unwrap();

        let res = branch_compare_upstream(repo_path, "test");

        assert_eq!(res.is_err(), true);
    }
}

#[cfg(test)]
mod tests_branches {
    use super::*;
    use crate::sync::{
        remotes::{get_remotes, push::push},
        rename_branch,
        tests::{
            debug_cmd_print, repo_clone, repo_init, repo_init_bare,
            write_commit_file,
        },
    };

    #[test]
    fn test_smoke() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert_eq!(
            get_branches_info(repo_path, true)
                .unwrap()
                .iter()
                .map(|b| b.name.clone())
                .collect::<Vec<_>>(),
            vec!["master"]
        );
    }

    #[test]
    fn test_multiple() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        create_branch(repo_path, "test").unwrap();

        assert_eq!(
            get_branches_info(repo_path, true)
                .unwrap()
                .iter()
                .map(|b| b.name.clone())
                .collect::<Vec<_>>(),
            vec!["master", "test"]
        );
    }

    fn clone_branch_commit_push(target: &str, branch_name: &str) {
        let (dir, repo) = repo_clone(target).unwrap();
        let dir = dir.path().to_str().unwrap();

        write_commit_file(&repo, "f1.txt", "foo", "c1");
        rename_branch(dir, "refs/heads/master", branch_name).unwrap();
        push(dir, "origin", branch_name, false, None, None).unwrap();
    }

    #[test]
    fn test_remotes_of_branches() {
        let (r1_path, _remote1) = repo_init_bare().unwrap();
        let (r2_path, _remote2) = repo_init_bare().unwrap();
        let (_r, repo) = repo_init().unwrap();

        let r1_path = r1_path.path().to_str().unwrap();
        let r2_path = r2_path.path().to_str().unwrap();

        //Note: create those test branches in our remotes
        clone_branch_commit_push(r1_path, "r1branch");
        clone_branch_commit_push(r2_path, "r2branch");

        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        //add the remotes
        repo.remote("r1", r1_path).unwrap();
        repo.remote("r2", r2_path).unwrap();

        //verify we got the remotes
        let remotes = get_remotes(repo_path).unwrap();
        assert_eq!(
            remotes,
            vec![String::from("r1"), String::from("r2")]
        );

        //verify we got only master right now
        let branches = get_branches_info(repo_path, true).unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].name, String::from("master"));

        //pull stuff from the two remotes
        debug_cmd_print(repo_path, "git pull r1");
        debug_cmd_print(repo_path, "git pull r2");

        //create local tracking branches
        debug_cmd_print(
            repo_path,
            "git checkout --track r1/r1branch",
        );
        debug_cmd_print(
            repo_path,
            "git checkout --track r2/r2branch",
        );

        let branches = get_branches_info(repo_path, true).unwrap();
        assert_eq!(branches.len(), 3);
        assert_eq!(
            branches[1]
                .local_details()
                .unwrap()
                .remote
                .as_ref()
                .unwrap(),
            "r1"
        );
        assert_eq!(
            branches[2]
                .local_details()
                .unwrap()
                .remote
                .as_ref()
                .unwrap(),
            "r2"
        );

        assert_eq!(
            get_branch_remote(repo_path, "r1branch")
                .unwrap()
                .unwrap(),
            String::from("r1")
        );

        assert_eq!(
            get_branch_remote(repo_path, "r2branch")
                .unwrap()
                .unwrap(),
            String::from("r2")
        );
    }

    #[test]
    fn test_branch_remote_no_upstream() {
        let (_r, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert_eq!(
            get_branch_remote(repo_path, "master").unwrap(),
            None
        );
    }

    #[test]
    fn test_branch_remote_no_branch() {
        let (_r, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert!(get_branch_remote(repo_path, "foo").is_err());
    }
}

#[cfg(test)]
mod tests_checkout {
    use super::*;
    use crate::sync::tests::repo_init;

    #[test]
    fn test_smoke() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        assert!(
            checkout_branch(repo_path, "refs/heads/master").is_ok()
        );
        assert!(
            checkout_branch(repo_path, "refs/heads/foobar").is_err()
        );
    }

    #[test]
    fn test_multiple() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        create_branch(repo_path, "test").unwrap();

        assert!(checkout_branch(repo_path, "refs/heads/test").is_ok());
        assert!(
            checkout_branch(repo_path, "refs/heads/master").is_ok()
        );
        assert!(checkout_branch(repo_path, "refs/heads/test").is_ok());
    }
}

#[cfg(test)]
mod test_delete_branch {
    use super::*;
    use crate::sync::tests::repo_init;

    #[test]
    fn test_delete_branch() {
        let (_td, repo) = repo_init().unwrap();
        let root = repo.path().parent().unwrap();
        let repo_path = root.as_os_str().to_str().unwrap();

        create_branch(repo_path, "branch1").unwrap();
        create_branch(repo_path, "branch2").unwrap();

        checkout_branch(repo_path, "refs/heads/branch1").unwrap();

        assert_eq!(
            repo.branches(None)
                .unwrap()
                .nth(1)
                .unwrap()
                .unwrap()
                .0
                .name()
                .unwrap()
                .unwrap(),
            "branch2"
        );

        delete_branch(repo_path, "refs/heads/branch2").unwrap();

        assert_eq!(
            repo.branches(None)
                .unwrap()
                .nth(1)
                .unwrap()
                .unwrap()
                .0
                .name()
                .unwrap()
                .unwrap(),
            "master"
        );
    }
}

#[cfg(test)]
mod test_remote_branches {
    use super::*;
    use crate::sync::remotes::push::push;
    use crate::sync::tests::{
        repo_clone, repo_init_bare, write_commit_file,
    };

    #[test]
    fn test_remote_branches() {
        let (r1_dir, _repo) = repo_init_bare().unwrap();

        let (clone1_dir, clone1) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone1_dir = clone1_dir.path().to_str().unwrap();

        // clone1

        write_commit_file(&clone1, "test.txt", "test", "commit1");

        push(clone1_dir, "origin", "master", false, None, None)
            .unwrap();

        create_branch(clone1_dir, "foo").unwrap();

        write_commit_file(&clone1, "test.txt", "test2", "commit2");

        push(clone1_dir, "origin", "foo", false, None, None).unwrap();

        // clone2

        let (clone2_dir, _clone2) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone2_dir = clone2_dir.path().to_str().unwrap();

        let local_branches =
            get_branches_info(clone2_dir, true).unwrap();

        assert_eq!(local_branches.len(), 1);

        let branches = get_branches_info(clone2_dir, false).unwrap();
        assert_eq!(dbg!(&branches).len(), 3);
        assert_eq!(&branches[0].name, "origin/HEAD");
        assert_eq!(&branches[1].name, "origin/foo");
        assert_eq!(&branches[2].name, "origin/master");
    }

    #[test]
    fn test_checkout_remote_branch() {
        let (r1_dir, _repo) = repo_init_bare().unwrap();

        let (clone1_dir, clone1) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();
        let clone1_dir = clone1_dir.path().to_str().unwrap();

        // clone1

        write_commit_file(&clone1, "test.txt", "test", "commit1");
        push(clone1_dir, "origin", "master", false, None, None)
            .unwrap();
        create_branch(clone1_dir, "foo").unwrap();
        write_commit_file(&clone1, "test.txt", "test2", "commit2");
        push(clone1_dir, "origin", "foo", false, None, None).unwrap();

        // clone2

        let (clone2_dir, _clone2) =
            repo_clone(r1_dir.path().to_str().unwrap()).unwrap();

        let clone2_dir = clone2_dir.path().to_str().unwrap();

        let local_branches =
            get_branches_info(clone2_dir, true).unwrap();

        assert_eq!(local_branches.len(), 1);

        let branches = get_branches_info(clone2_dir, false).unwrap();

        // checkout origin/foo
        checkout_remote_branch(clone2_dir, &branches[1]).unwrap();

        assert_eq!(
            get_branches_info(clone2_dir, true).unwrap().len(),
            2
        );

        assert_eq!(&get_branch_name(clone2_dir).unwrap(), "foo");
    }
}
