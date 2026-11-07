use hydrogen::{contracts::ApiError, launcher::tasks::Tasks};

#[test]
fn download_progress_tracks_partial_bytes_retry_resets_and_verified_files() {
    let tasks = Tasks::default();
    let job = tasks.start("install", Some("instance")).unwrap();
    job.begin_download(
        [
            (".minecraft/client.jar".into(), Some(1000)),
            (".minecraft/library.jar".into(), Some(100)),
        ],
        true,
    );
    job.file_started(".minecraft/client.jar");
    std::thread::sleep(std::time::Duration::from_millis(110));
    job.file_progress(".minecraft/client.jar", 600, false);
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(
        (snapshot.completed, snapshot.total, snapshot.completed_files),
        (600, Some(1100), 0)
    );
    assert_eq!(snapshot.active_files, ["client.jar"]);
    job.file_progress(".minecraft/client.jar", 0, false);
    job.file_progress(".minecraft/client.jar", 1000, true);
    job.file_progress(".minecraft/library.jar", 100, true);
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(
        (
            snapshot.completed,
            snapshot.completed_files,
            snapshot.total_files
        ),
        (1100, 2, 2)
    );
    assert!(snapshot.active_files.is_empty());
    job.begin_download([("Fabric library".into(), None)], true);
    job.file_progress("Fabric library", 250, true);
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(
        (snapshot.completed, snapshot.total, snapshot.completed_files),
        (250, Some(250), 1)
    );
    job.begin_download([("assets/indexes/5.json".into(), None)], false);
    job.file_progress("assets/indexes/5.json", 1024, true);
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(
        (
            snapshot.completed,
            snapshot.completed_files,
            snapshot.total_files
        ),
        (1024, 0, 0)
    );
}

#[test]
fn processor_phase_clears_previous_transfers_and_a_new_plan_tracks_only_its_files() {
    let tasks = Tasks::default();
    let job = tasks.start("install", Some("instance")).unwrap();
    job.phase("downloading", "libraries");
    job.begin_download([("libraries/fastutil.jar".into(), None)], true);
    job.file_started("libraries/fastutil.jar");
    assert_eq!(
        job.state
            .lock()
            .unwrap()
            .download
            .as_ref()
            .unwrap()
            .active_files,
        ["libraries\\fastutil.jar"]
    );
    job.phase("processing", "processor step 1/2");
    assert!(
        job.state
            .lock()
            .unwrap()
            .download
            .as_ref()
            .unwrap()
            .active_files
            .is_empty()
    );

    job.phase("verifying", "verify merged game files");
    job.begin_download([("libraries/new-loader.jar".into(), None)], true);
    job.file_started("libraries/fastutil.jar");
    job.file_started("libraries/new-loader.jar");
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(snapshot.total_files, 1);
    assert_eq!(snapshot.active_files, ["libraries\\new-loader.jar"]);
    job.phase("committing", "save instance");
    assert!(
        job.state
            .lock()
            .unwrap()
            .download
            .as_ref()
            .unwrap()
            .active_files
            .is_empty()
    );
}

#[test]
fn replayed_file_started_does_not_reactivate_a_verified_file() {
    let tasks = Tasks::default();
    let job = tasks.start("install", Some("instance")).unwrap();
    job.begin_download(
        [("completed.jar".into(), None), ("pending.jar".into(), None)],
        true,
    );
    job.file_progress("completed.jar", 100, true);
    job.file_started("completed.jar");
    job.file_started("pending.jar");
    let snapshot = job.state.lock().unwrap().download.clone().unwrap();
    assert_eq!(snapshot.completed_files, 1);
    assert_eq!(snapshot.active_files, ["pending.jar"]);
}

#[test]
fn retry_replaces_the_failed_install_and_directory_switch_forgets_downloads() {
    let tasks = Tasks::default();
    let first = tasks.start("install", Some("instance")).unwrap();
    assert!(tasks.start("install", Some("instance")).is_err());
    first.finish(Err(ApiError::new("NETWORK", "connection lost")));
    let retry = tasks.start("install", Some("instance")).unwrap();
    assert_eq!(tasks.list().len(), 1);
    assert_ne!(
        retry.state.lock().unwrap().id,
        first.state.lock().unwrap().id
    );
    let id = retry.state.lock().unwrap().id.clone();
    tasks.cancel(&id).unwrap();
    retry.finish(retry.check());
    assert_eq!(retry.state.lock().unwrap().phase, "cancelled");
    tasks.forget_downloads(None);
    assert!(tasks.list().is_empty());
}
