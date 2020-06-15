//! Management of the directory layout of the cache
//!
//! The directory layout is a little tricky at times, hence a separate file to
//! house this logic. The current layout looks like this:
//!
//! ```text
//! # This is the root directory for all cache output
//! cache/
//!
//!     A cache version to allow breaking changes in the cache structure.
//!     $CACHE_VERSION/
//!
//!         # File used to lock the directory to prevent multiple cargo processes
//!         # from using it at the same time.
//!         .cargo-lock
//!
//!         # Hidden directory that holds all of the fingerprint files for all
//!         # packages
//!         .fingerprint/
//!             # Each package is in a separate directory.
//!             # Note that different target kinds have different filename prefixes.
//!             $pkgname-$META/
//!                 # Set of source filenames for this package.
//!                 dep-lib-$targetname
//!                 # Timestamp when this package was last built.
//!                 invoked.timestamp
//!                 # The fingerprint hash.
//!                 lib-$targetname
//!                 # Detailed information used for logging the reason why
//!                 # something is being recompiled.
//!                 lib-$targetname.json
//!                 # The console output from the compiler. This is cached
//!                 # so that warnings can be redisplayed for "fresh" units.
//!                 output-lib-$targetname
//!
//!         # This is the root directory for all rustc artifacts except build
//!         # scripts, examples, and test and bench executables. Almost every
//!         # artifact should have a metadata hash added to its filename to
//!         # prevent collisions. One notable exception is dynamic libraries.
//!         deps/
//!
//!         # This is the location at which the output of all custom build
//!         # commands are rooted.
//!         build/
//!
//!             # Each package gets its own directory where its build script and
//!             # script output are placed
//!             $pkgname-$META/    # For the build script itself.
//!                 # The build script executable (name may be changed by user).
//!                 build-script-build-$META
//!                 # Hard link to build-script-build-$META.
//!                 build-script-build
//!                 # Dependency information generated by rustc.
//!                 build-script-build-$META.d
//!                 # Debug information, depending on platform and profile
//!                 # settings.
//!                 <debug symbols>
//!
//!             # The package shows up twice with two different metadata hashes.
//!             $pkgname-$META/  # For the output of the build script.
//!                 # Timestamp when the build script was last executed.
//!                 invoked.timestamp
//!                 # Directory where script can output files ($OUT_DIR).
//!                 out/
//!                 # Output from the build script.
//!                 output
//!                 # Path to `out`, used to help when the target directory is
//!                 # moved.
//!                 root-output
//!                 # Stderr output from the build script.
//!                 stderr
//! ```

use crate::core::compiler::Context;
use crate::util::paths;
use crate::util::{CargoResult, FileLock, Filesystem};
use std::path::{Path, PathBuf};

/// The cache version, make sure to increment this if you make any
/// breaking changes to the cache folder!
const CACHE_VERSION: &str = "0";

/// Contains the paths of all cache output locations.
///
/// See module docs for more information.
pub struct CacheLayout {
    /// The root directory: most likely `$CARGO_HOME/.cargo/cache`
    root: PathBuf,
    /// The directory for the current cache version: `$root/$VERSION`
    dest: PathBuf,
    /// The directory with rustc artifacts: `$dest/deps`
    deps: PathBuf,
    /// The directory for build scripts: `$dest/build`
    build: PathBuf,
    /// The directory for fingerprints: `$dest/.fingerprint`
    fingerprint: PathBuf,
    /// The lockfile for the cache (`.cargo-lock`). Will be unlocked when this
    /// struct is `drop`ped.
    _lock: FileLock,
}

impl CacheLayout {
    /// Calculate the paths for cache output, lock the cache directory, and return as a CacheLayout.
    ///
    /// This function will block if the directory is already locked.
    pub fn new(
        cx: &Context<'_, '_>,
    ) -> CargoResult<Option<CacheLayout>> {
        if let Some(root) = &cx.bcx.config.cache_dir()? {
            // let mut root = ws.target_dir();
            let root = root.clone();
            let dest = root.join(CACHE_VERSION);
            // If the root directory doesn't already exist go ahead and create it
            // here. Use this opportunity to exclude it from backups as well if the
            // system supports it since this is a freshly created folder.
            if !dest.as_path_unlocked().exists() {
                dest.create_dir()?;
                exclude_from_backups(dest.as_path_unlocked());
            }

            // For now we don't do any more finer-grained locking on the artifact
            // directory, so just lock the entire thing for the duration of this
            // compile.
            let lock = dest.open_rw(".cargo-lock", cx.bcx.config, "build directory")?;
            let root = root.into_path_unlocked();
            let dest = dest.into_path_unlocked();

            Ok(Some(CacheLayout {
                deps: dest.join("deps"),
                build: dest.join("build"),
                fingerprint: dest.join(".fingerprint"),
                root,
                dest,
                _lock: lock,
            }))
        } else {
            Ok(None)
        }
    }

    /// Makes sure all directories stored in the Layout exist on the filesystem.
    pub fn prepare(&mut self) -> CargoResult<()> {
        paths::create_dir_all(&self.deps)?;
        paths::create_dir_all(&self.fingerprint)?;
        paths::create_dir_all(&self.build)?;

        Ok(())
    }

    /// Fetch the directory for the curent cache version.
    pub fn dest(&self) -> &Path {
        &self.dest
    }
    /// Fetch the deps path.
    pub fn deps(&self) -> &Path {
        &self.deps
    }
    /// Fetch the root path (`/…/target`).
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Fetch the fingerprint path.
    pub fn fingerprint(&self) -> &Path {
        &self.fingerprint
    }
    /// Fetch the build script path.
    pub fn build(&self) -> &Path {
        &self.build
    }
}

#[cfg(not(target_os = "macos"))]
fn exclude_from_backups(_: &Path) {}

#[cfg(target_os = "macos")]
/// Marks files or directories as excluded from Time Machine on macOS
///
/// This is recommended to prevent derived/temporary files from bloating backups.
fn exclude_from_backups(path: &Path) {
    use core_foundation::base::TCFType;
    use core_foundation::{number, string, url};
    use std::ptr;

    // For compatibility with 10.7 a string is used instead of global kCFURLIsExcludedFromBackupKey
    let is_excluded_key: Result<string::CFString, _> = "NSURLIsExcludedFromBackupKey".parse();
    let path = url::CFURL::from_path(path, false);
    if let (Some(path), Ok(is_excluded_key)) = (path, is_excluded_key) {
        unsafe {
            url::CFURLSetResourcePropertyForKey(
                path.as_concrete_TypeRef(),
                is_excluded_key.as_concrete_TypeRef(),
                number::kCFBooleanTrue as *const _,
                ptr::null_mut(),
            );
        }
    }
    // Errors are ignored, since it's an optional feature and failure
    // doesn't prevent Cargo from working
}
