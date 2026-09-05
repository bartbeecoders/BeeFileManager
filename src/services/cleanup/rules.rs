// SPDX-License-Identifier: GPL-3.0-or-later

//! Rules that decide whether a directory is a regenerable build/cache artifact.
//!
//! A rule only fires when the directory *name* matches **and** the required
//! marker files are present, so a folder called `build` full of holiday photos
//! is never flagged just because of its name.

use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Confidence {
    /// Unambiguous tool output. Deleting it costs you a rebuild, nothing else.
    High,
    /// Almost always generated, but the name is shared with hand-written dirs.
    Medium,
    /// Plausible, but check it yourself before removing.
    Low,
}

impl Confidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// How a directory name is recognised.
#[derive(Clone, Copy)]
pub enum NameMatch {
    /// Exact name, compared case-insensitively.
    Exact(&'static str),
    /// Name starts with this, e.g. `cmake-build-`.
    Prefix(&'static str),
    /// Name ends with this, e.g. `.egg-info`.
    Suffix(&'static str),
    /// The directory's full path ends with this, e.g. `.cargo/registry`.
    PathSuffix(&'static str),
}

pub struct Rule {
    pub name: NameMatch,
    /// At least one of these entries must sit next to the directory.
    /// `*.ext` matches any sibling with that extension. Empty = no requirement.
    pub sibling_any: &'static [&'static str],
    /// All of these entries must sit next to the directory.
    pub sibling_all: &'static [&'static str],
    /// At least one of these entries must exist *inside* the directory.
    pub inner_any: &'static [&'static str],
    /// Human label: what the directory is.
    pub kind: &'static str,
    pub confidence: Confidence,
    /// How the user gets it back.
    pub regen: &'static str,
}

const fn rule(
    name: NameMatch,
    kind: &'static str,
    confidence: Confidence,
    regen: &'static str,
) -> Rule {
    Rule {
        name,
        sibling_any: &[],
        sibling_all: &[],
        inner_any: &[],
        kind,
        confidence,
        regen,
    }
}

const fn with_sibling(mut r: Rule, sibling_any: &'static [&'static str]) -> Rule {
    r.sibling_any = sibling_any;
    r
}

use Confidence::{High, Low, Medium};
use NameMatch::{Exact, PathSuffix, Prefix, Suffix};

/// Markers that identify "there is a JS project here".
const JS: &[&str] = &["package.json", "deno.json", "bun.lockb"];
const PY: &[&str] = &["pyproject.toml", "setup.py", "setup.cfg", "tox.ini"];
const DOTNET: &[&str] = &["*.csproj", "*.fsproj", "*.vbproj", "*.sln", "*.slnx"];
const CMAKE: &[&str] = &[
    "CMakeLists.txt",
    "meson.build",
    "configure.ac",
    "Makefile.am",
    "CMakePresets.json",
];

pub static RULES: &[Rule] = &[
    // ---- JavaScript / TypeScript ----------------------------------------
    rule(Exact("node_modules"), "npm packages", High, "npm install"),
    rule(
        Exact("bower_components"),
        "bower packages",
        High,
        "bower install",
    ),
    rule(Exact(".next"), "Next.js build", High, "next build"),
    rule(Exact(".nuxt"), "Nuxt build", High, "nuxt build"),
    rule(Exact(".output"), "Nuxt/Nitro output", High, "nuxt build"),
    rule(Exact(".svelte-kit"), "SvelteKit build", High, "vite build"),
    rule(Exact(".astro"), "Astro build", High, "astro build"),
    rule(Exact(".angular"), "Angular cache", High, "ng build"),
    rule(
        Exact(".docusaurus"),
        "Docusaurus cache",
        High,
        "docusaurus build",
    ),
    rule(Exact(".parcel-cache"), "Parcel cache", High, "parcel build"),
    rule(Exact(".turbo"), "Turborepo cache", High, "turbo run build"),
    rule(Exact(".vite"), "Vite cache", High, "vite build"),
    rule(Exact(".webpack"), "webpack cache", High, "webpack"),
    rule(Exact(".rollup.cache"), "Rollup cache", High, "rollup -c"),
    rule(Exact(".vercel"), "Vercel build", High, "vercel build"),
    rule(Exact(".netlify"), "Netlify build", High, "netlify build"),
    rule(Exact(".expo"), "Expo cache", High, "expo start"),
    rule(Exact(".pnpm-store"), "pnpm store", High, "pnpm install"),
    rule(Exact(".yarn"), "Yarn cache/state", Medium, "yarn install"),
    rule(Exact(".nyc_output"), "coverage data", High, "re-run tests"),
    rule(
        Exact("playwright-report"),
        "Playwright report",
        High,
        "re-run tests",
    ),
    rule(Exact("test-results"), "test output", Medium, "re-run tests"),
    with_sibling(
        rule(
            Exact("storybook-static"),
            "Storybook build",
            High,
            "build-storybook",
        ),
        JS,
    ),
    with_sibling(
        rule(Exact("dist"), "build output", Medium, "npm run build"),
        JS,
    ),
    with_sibling(
        rule(Exact("out"), "build output", Medium, "npm run build"),
        JS,
    ),
    with_sibling(
        rule(Exact(".cache"), "build cache", Medium, "npm run build"),
        JS,
    ),
    with_sibling(
        rule(Exact("coverage"), "coverage report", Medium, "re-run tests"),
        JS,
    ),
    // ---- Rust -----------------------------------------------------------
    with_sibling(
        rule(Exact("target"), "Rust build output", High, "cargo build"),
        &["Cargo.toml"],
    ),
    // ---- Python ---------------------------------------------------------
    rule(
        Exact("__pycache__"),
        "Python bytecode",
        High,
        "regenerated on import",
    ),
    rule(
        Exact(".pytest_cache"),
        "pytest cache",
        High,
        "re-run pytest",
    ),
    rule(Exact(".mypy_cache"), "mypy cache", High, "re-run mypy"),
    rule(Exact(".ruff_cache"), "ruff cache", High, "re-run ruff"),
    rule(Exact(".pytype"), "pytype cache", High, "re-run pytype"),
    rule(Exact(".pyre"), "pyre cache", High, "re-run pyre"),
    rule(Exact(".hypothesis"), "hypothesis DB", High, "re-run tests"),
    rule(Exact(".tox"), "tox environments", High, "tox"),
    rule(Exact(".nox"), "nox environments", High, "nox"),
    rule(Exact(".eggs"), "setuptools eggs", High, "pip install -e ."),
    rule(
        Suffix(".egg-info"),
        "package metadata",
        High,
        "pip install -e .",
    ),
    rule(Exact("htmlcov"), "coverage report", High, "re-run coverage"),
    rule(
        Exact(".ipynb_checkpoints"),
        "notebook checkpoints",
        High,
        "n/a",
    ),
    Rule {
        inner_any: &["pyvenv.cfg"],
        ..rule(
            Exact(".venv"),
            "Python virtualenv",
            High,
            "python -m venv .venv",
        )
    },
    Rule {
        inner_any: &["pyvenv.cfg"],
        ..rule(
            Exact("venv"),
            "Python virtualenv",
            High,
            "python -m venv venv",
        )
    },
    Rule {
        inner_any: &["pyvenv.cfg"],
        ..rule(
            Exact("env"),
            "Python virtualenv",
            High,
            "python -m venv env",
        )
    },
    Rule {
        inner_any: &["pyvenv.cfg"],
        ..rule(
            Exact(".env"),
            "Python virtualenv",
            High,
            "python -m venv .env",
        )
    },
    with_sibling(
        rule(
            Exact("build"),
            "Python build dir",
            Medium,
            "python -m build",
        ),
        PY,
    ),
    with_sibling(
        rule(
            Exact("dist"),
            "Python wheels/sdist",
            Medium,
            "python -m build",
        ),
        PY,
    ),
    // ---- C / C++ / CMake ------------------------------------------------
    rule(Exact("CMakeFiles"), "CMake internals", High, "cmake ."),
    rule(
        Prefix("cmake-build-"),
        "CLion build dir",
        High,
        "rebuild in IDE",
    ),
    rule(Exact(".ccls-cache"), "ccls index", High, "reindex"),
    rule(PathSuffix(".cache/clangd"), "clangd index", High, "reindex"),
    rule(Exact(".ccache"), "ccache", High, "recompiles"),
    with_sibling(
        rule(
            Exact("build"),
            "CMake/Meson build dir",
            Medium,
            "cmake --build .",
        ),
        CMAKE,
    ),
    with_sibling(
        rule(Exact("builddir"), "Meson build dir", High, "meson compile"),
        CMAKE,
    ),
    with_sibling(rule(Exact("_build"), "build dir", Medium, "rebuild"), CMAKE),
    // ---- .NET -----------------------------------------------------------
    with_sibling(
        rule(Exact("bin"), ".NET build output", Medium, "dotnet build"),
        DOTNET,
    ),
    with_sibling(
        rule(Exact("obj"), ".NET intermediates", High, "dotnet build"),
        DOTNET,
    ),
    with_sibling(
        rule(Exact("Debug"), "MSBuild output", Medium, "rebuild"),
        DOTNET,
    ),
    with_sibling(
        rule(Exact("Release"), "MSBuild output", Medium, "rebuild"),
        DOTNET,
    ),
    rule(Exact(".vs"), "Visual Studio cache", High, "reopen solution"),
    // ---- JVM ------------------------------------------------------------
    with_sibling(
        rule(Exact("target"), "Maven build output", High, "mvn package"),
        &["pom.xml"],
    ),
    with_sibling(
        rule(Exact("build"), "Gradle build output", High, "gradle build"),
        &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
    ),
    rule(
        Exact(".gradle"),
        "Gradle project cache",
        High,
        "gradle build",
    ),
    rule(Exact(".kotlin"), "Kotlin session data", High, "rebuild"),
    with_sibling(
        rule(Exact("out"), "IntelliJ output", Medium, "rebuild in IDE"),
        &[".idea"],
    ),
    with_sibling(
        rule(Exact("target"), "sbt build output", High, "sbt compile"),
        &["build.sbt"],
    ),
    // ---- Go / PHP / Ruby ------------------------------------------------
    with_sibling(
        rule(Exact("vendor"), "vendored Go modules", Low, "go mod vendor"),
        &["go.mod"],
    ),
    with_sibling(
        rule(
            Exact("vendor"),
            "Composer packages",
            Medium,
            "composer install",
        ),
        &["composer.json"],
    ),
    with_sibling(
        rule(
            Exact(".bundle"),
            "Bundler config/cache",
            Low,
            "bundle install",
        ),
        &["Gemfile"],
    ),
    // ---- Mobile / game engines ------------------------------------------
    with_sibling(
        rule(Exact("Pods"), "CocoaPods", High, "pod install"),
        &["Podfile"],
    ),
    rule(
        Exact("DerivedData"),
        "Xcode derived data",
        High,
        "rebuild in Xcode",
    ),
    with_sibling(
        rule(
            Exact(".dart_tool"),
            "Dart tool cache",
            High,
            "flutter pub get",
        ),
        &["pubspec.yaml"],
    ),
    with_sibling(
        rule(Exact("build"), "Flutter build", High, "flutter build"),
        &["pubspec.yaml"],
    ),
    with_sibling(
        rule(Exact(".godot"), "Godot cache", High, "reopen project"),
        &["project.godot"],
    ),
    with_sibling(
        rule(Exact(".import"), "Godot imports", High, "reopen project"),
        &["project.godot"],
    ),
    with_sibling(
        rule(Exact("Binaries"), "Unreal binaries", High, "rebuild"),
        &["*.uproject"],
    ),
    with_sibling(
        rule(
            Exact("Intermediate"),
            "Unreal intermediates",
            High,
            "rebuild",
        ),
        &["*.uproject"],
    ),
    with_sibling(
        rule(Exact("DerivedDataCache"), "Unreal DDC", High, "rebuild"),
        &["*.uproject"],
    ),
    Rule {
        sibling_all: &["Assets", "ProjectSettings"],
        ..rule(
            Exact("Library"),
            "Unity library cache",
            High,
            "reopen project",
        )
    },
    Rule {
        sibling_all: &["Assets", "ProjectSettings"],
        ..rule(Exact("Temp"), "Unity temp files", High, "reopen project")
    },
    Rule {
        sibling_all: &["Assets", "ProjectSettings"],
        ..rule(Exact("Logs"), "Unity logs", High, "reopen project")
    },
    Rule {
        sibling_all: &["Assets", "ProjectSettings"],
        ..rule(Exact("Obj"), "Unity intermediates", High, "reopen project")
    },
    // ---- Other toolchains -----------------------------------------------
    with_sibling(
        rule(Exact(".build"), "Swift build", High, "swift build"),
        &["Package.swift"],
    ),
    rule(Exact(".stack-work"), "Stack build", High, "stack build"),
    rule(Exact("dist-newstyle"), "Cabal build", High, "cabal build"),
    with_sibling(
        rule(Exact("_build"), "Elixir/Erlang build", High, "mix compile"),
        &["mix.exs", "rebar.config"],
    ),
    with_sibling(
        rule(Exact("deps"), "Elixir deps", High, "mix deps.get"),
        &["mix.exs"],
    ),
    rule(Exact("zig-cache"), "Zig cache", High, "zig build"),
    rule(Exact(".zig-cache"), "Zig cache", High, "zig build"),
    rule(Exact("zig-out"), "Zig output", High, "zig build"),
    rule(Exact("nimcache"), "Nim cache", High, "nim c"),
    rule(
        Exact(".terraform"),
        "Terraform providers",
        High,
        "terraform init",
    ),
    rule(
        Exact(".serverless"),
        "Serverless build",
        High,
        "serverless package",
    ),
    rule(Exact(".aws-sam"), "SAM build", High, "sam build"),
    with_sibling(
        rule(Exact("_site"), "Jekyll site", High, "jekyll build"),
        &["_config.yml"],
    ),
    // ---- Machine-wide tool caches (matched by path) ----------------------
    rule(
        PathSuffix(".cargo/registry"),
        "cargo registry cache",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".cargo/git"),
        "cargo git checkouts",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".npm/_cacache"),
        "npm cache",
        High,
        "npm cache verify",
    ),
    rule(
        PathSuffix(".cache/yarn"),
        "Yarn cache",
        High,
        "re-downloaded",
    ),
    rule(PathSuffix(".cache/pip"), "pip cache", High, "re-downloaded"),
    rule(PathSuffix(".cache/uv"), "uv cache", High, "re-downloaded"),
    rule(
        PathSuffix(".cache/go-build"),
        "Go build cache",
        High,
        "go build",
    ),
    rule(
        PathSuffix("go/pkg/mod"),
        "Go module cache",
        Medium,
        "go mod download",
    ),
    rule(
        PathSuffix(".gradle/caches"),
        "Gradle cache",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".m2/repository"),
        "Maven repository",
        Low,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".nuget/packages"),
        "NuGet packages",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".pub-cache"),
        "Dart pub cache",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".cache/ms-playwright"),
        "Playwright browsers",
        Medium,
        "playwright install",
    ),
    rule(
        PathSuffix(".cache/puppeteer"),
        "Puppeteer browsers",
        Medium,
        "re-downloaded",
    ),
    rule(
        PathSuffix(".cache/JetBrains"),
        "JetBrains caches",
        Medium,
        "regenerated by IDE",
    ),
    rule(
        PathSuffix(".cache/bazel"),
        "Bazel cache",
        High,
        "bazel build",
    ),
    rule(PathSuffix(".gem"), "Ruby gems cache", Low, "gem install"),
];

/// Directories we always refuse to descend into, whatever the rules say.
pub const NEVER_DESCEND: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".bzr",
    "lost+found",
    ".Trash",
    ".Trash-1000",
    "System Volume Information",
];

/// Hidden directories that still need walking so path-suffix cache rules can fire.
pub const HIDDEN_STORES: &[&str] = &[
    ".cargo",
    ".npm",
    ".cache",
    ".gradle",
    ".m2",
    ".nuget",
    ".pub-cache",
    ".gem",
    ".local",
];

/// Absolute paths that are never worth walking (pseudo filesystems, mounts).
pub const NEVER_SCAN: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/run/user",
    "/var/run",
    "/var/lock",
];

fn eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.eq_ignore_ascii_case(b)
}

/// Does `entries` contain something matching `marker` (`*.ext` allowed)?
fn has_marker(entries: &HashSet<String>, marker: &str) -> bool {
    if let Some(ext) = marker.strip_prefix("*.") {
        entries.iter().any(|e| {
            e.rsplit_once('.')
                .is_some_and(|(_, e)| eq_ignore_case(e, ext))
        })
    } else {
        entries.iter().any(|e| eq_ignore_case(e, marker))
    }
}

fn name_matches(m: NameMatch, dir_name: &str, dir_path: &Path) -> bool {
    match m {
        NameMatch::Exact(n) => eq_ignore_case(dir_name, n),
        NameMatch::Prefix(p) => {
            dir_name.len() > p.len() && dir_name[..p.len()].eq_ignore_ascii_case(p)
        }
        NameMatch::Suffix(s) => {
            dir_name.len() > s.len() && dir_name[dir_name.len() - s.len()..].eq_ignore_ascii_case(s)
        }
        NameMatch::PathSuffix(s) => dir_path.to_string_lossy().ends_with(s),
    }
}

/// Classify `dir_path`. `siblings` is the set of entry names in its parent
/// (files *and* directories), which is what the marker checks look at.
pub fn classify(
    dir_path: &Path,
    dir_name: &str,
    siblings: &HashSet<String>,
) -> Option<&'static Rule> {
    RULES
        .iter()
        .filter(|r| name_matches(r.name, dir_name, dir_path))
        .filter(|r| {
            r.sibling_any.is_empty() || r.sibling_any.iter().any(|m| has_marker(siblings, m))
        })
        .filter(|r| r.sibling_all.iter().all(|m| has_marker(siblings, m)))
        .filter(|r| r.inner_any.is_empty() || r.inner_any.iter().any(|m| dir_path.join(m).exists()))
        // Several rules can match the same directory (a `target` next to both
        // Cargo.toml and pom.xml); the most confident one wins.
        .min_by_key(|r| r.confidence)
}

#[cfg(test)]
mod tests;
