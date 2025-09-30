# Build Script Parsing and Caching Strategy

## The Problem

We have a chicken-and-egg problem with caching build artifacts:

1. **To back up artifacts**: We need cache keys derived from build script outputs
2. **To restore artifacts**: We need to reconstruct expected cache keys WITHOUT running build scripts

This is critical because some build scripts compile C libraries, taking 10s-100s of seconds.
If we can't cache these effectively, it severely weakens the value proposition of `hurry`.

> [!NOTE]
> This is a toy example.
> The real build script is much more complicated, because of course it is:
> https://github.com/sfackler/rust-openssl/blob/master/openssl-sys/build/main.rs

```rust
// Inside build.rs
fn main() {
    let openssl_dir = find_openssl(); // System-dependent!
    let openssl = openssl_dir.join("openssl");
    println!("cargo:rerun-if-changed={openssl}");
    println!("cargo:include={openssl_dir}");
    println!("cargo:rustc-link-search=native={openssl_dir}");
    println!("cargo:rustc-link-lib=native={openssl}");
}
```

We need `openssl_dir` to compute the cache key, but `find_openssl()` requires running the build script, which requires cache keys to know what to restore or actually running the build script. We want to avoid running the build script when possible since they can take 10's to 100's of seconds to build.

> [!TIP]
> One example is [`aws-lc-sys`](https://github.com/aws/aws-lc-rs/blob/main/aws-lc-sys/builder/main.rs), which on my system (MacBook Pro with an M4 Pro SoC) takes 19 seconds to build.

## Static analysis difficulties

Fully modeling build scripts is effectively equivalent to modeling a full execution environment, as build scripts can execute any arbitrary code; this is sort of obviously intractable. At the same time we can't e.g. simply cache key off of the hash of the build script itself, because an extremely common use case for build scripts is to find locations of system dependencies; these will have the same build script file but different inputs.

At the same time, it is our opinion that the vast majority of build scripts likely are quite basic and exist mainly to set environment variables for the build or to run some form of codegen not terribly more complicated than a procedural macro.

Given this we need to find a level of static analysis that allows us to model a subset of build scripts that we can tractably introspect, along with some form of generic "bailing out" caching for the rest. The subset of build scripts will be almost embarassingly small to begin but we can grow it over time.

## Proposed Approach

We care about a small set of "kinds" of crates:
1. Crates without a build script at all.
2. Crates with a "const" build script.
3. Crates with a "pure" build script.
4. Crates with a "side-effectful" build script.

In all cases, what we're really after is the _list of inputs_ to the build script. For example, consider this toy build script:

```rust
// Toy `build.rs` example, say it's inside `toyssl-sys@1.2.3`
fn main() {
    let should_vendor = std::env::var("TOYSSL_VENDOR") == Some("1");
    let openssl_dir = if should_vendor {
      unpack_temp(include_bytes("openssl-src.tar.gz"))
    } else {
      find_system_openssl()
    };

    let openssl = openssl_dir.join("openssl");
    println!("cargo:rerun-if-changed={openssl}");
    println!("cargo:include={openssl_dir}");
    println!("cargo:rustc-link-search=native={openssl_dir}");
    println!("cargo:rustc-link-lib=native={openssl}");
}
```

We really want to know "what are all possible inputs for this build script". Once we know the list of inputs, we can record those in the cache at the time we back up the output of the build script; imagine a system where we can resolve the inputs and then record something like this for the above build script:

> `toyssl-sys@1.2.3` <- [
>   Input::Env("TOYSSL_VENDOR"),
>   Input::File("$HOME/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/toyssl-1.2.3/openssl-src.tar.gz"),
>   Input::Dir("/opt/homebrew/lib/"),
>   Input::File("/opt/homebrew/lib/libssl.a"),
> ]

Then when we go to restore `toyssl-sys@1.2.3` artifacts, we can query the cache for "what are the inputs" and then evaluate them (along with our existing cache key data for other library information) to form a cache key, then try to restore that cache key. The pseudocode algorithm for deriving the key used to store or restore the artifact(s) for the build script would be:

```
fn build_script_artifact_key(library_name, library_version, base_key, path) {
  // We classify the kind of build script by evaluating the on-disk location for
  // the library.
  let kind = classify(path);

  // "base_key" is a standin for arbitrary opaque data determined earlier in the
  // cache process; for example this could communicate things like target triple
  // or CPU instruction set, etc.
  let inputs = list_inputs(library_name, library_version, base_key, kind);

  // The first few fields allow differentiation based on "baseline" information,
  // since not all build scripts have inputs. We differentiate by "kind" here
  // so that if a crate switches build script kinds without actually changing
  // its inputs they're under different keys.
  let fields = [
    library_name,
    library_version,
    base_key,
    "build-script",
    kind,
  ];

  for input in inputs {
    let value = get_value(input);
    fields.push(escape(input) + "=" + escape(value));
  }

  // We may actually treat the various fields as keys to get the cache artifacts
  // out of the database instead of literally a hashed key, that's all
  // implementation detail.
  return hash(fields.join(";"));
}
```

The kinds of build scripts are classified using a fallthrough classifier: the system runs classifiers on the crate being compiled in order. The first successful validation classifies the kind of the build script; failing validation "falls through" to the next classifier. The final classifier always validates, so it is a "catch all".

The "kinds" of crates all classify slightly differently and usually have different sets of inputs:
1. Crates without a build script at all: classified by lack of `build.rs` file or `build` key in `Cargo.toml`.
2. Crates with a "const" build script: classified by lack of input-reading primitives like `std::fs` or `std::env`.
3. Crates with a "pure" build script: classified by being deterministic given the same relevant environment.
  - Note: to start with for now we'll infer the purity of a build script via precomputation; keep reading for details.
4. Crates with a "side-effectful" build script: everything else.

The first and second forms are easy; either no script or a script with no inputs. "Const" build scripts are likely extremely rare though, so we may even just implement this as "if the crate imports anything, it's not const". We'll leave that up to implementation detail.

Differentiating "pure" vs "side-effectful" is quite hard though- we'd have to implement non-trivial parsing even for our toy example above, to say nothing of real-world build scripts like those used in [`ring`](https://github.com/briansmith/ring/blob/main/build.rs) or [`openssl`](https://github.com/sfackler/rust-openssl/blob/master/openssl-sys/build/main.rs).

What we'll do to start with is a brute force approach: we'll precompute known crates!

> [!NOTE]
> Precomputation is not the ideal end-goal for us; it only works for third party crates on public sources such as `crates.io` and we'd ideally support _any_ crate. But we're targeting this to begin with.

We'll pre-evaluate all crates on `crates.io` against the classification system described above. For crates that could fall into the "pure" or "side-effectful" classifications (meaning: crates that have a build script but it is not provably a "const" build script) the system will try to do the following:

1. Set up a system to build and install the dependency. I'm not sure how we'll do this exactly but vaguely I suspect we can use the CI configuration in the repo and/or use an LLM to configure a test project.
2. Run the build in a few different configurations, changing the configuration of the build environment for each test. Examples:
  - Test the build in different directories
  - Test the build at different timestamps
  - Test the build consecutively in the same container
  - Test the build on different containers of the same OS/distribution
  - Test the build on different containers with different OS/distribution
  - Others as we think of them
3. Check whether the output is the same; if so then the build script is "pure". "The same output" here is defined as the output of the build script in `stdio` pipes and the contents of the build script `out/` directory, not the compiled build script itself being byte for byte equal.

> [!NOTE]
> "pure" and "side-effectful" build scripts are always cache keyed by the OS on which it's being built.

For "pure" build scripts discovered through this method, the build script "inputs" for the purpose of caching will just be the hash of the contents of the build script. Ideally we'd enumerate the inputs somehow but the precomputation approach is meant to allow us to sidestep that requirement. For "side-effectful" scripts, the list of inputs will also include the cached contents of the build script, but additionally will include a set of keys that we'll try to ensure only reflect this environment- for example we'll try to set it up such that these builds could be reused across multiple runs of the same CI job, but not across machines or distinct CI jobs.

The specific steps we'll take aren't yet known, but this is the overall gist.

> [!TIP]
> Note that an invalid cache hit won't lead to an incorrect build; it'll just waste time as we'll restore the outputs and then Cargo will ignore them. So it's okay if this is best-effort.

## Future enhancements

There are a few future enhancements we can work towards for a more robust caching system, here are a few (but there are probably many more):
- Better categorization
- Better modeling, especially for "pure" build scripts, without relying on precomputation
- Better tracking of inputs for "pure" build scripts during precomputation (e.g. using `strace` to track files read)
- We might want to add specific handlers for popular crates. This might be something LLMs can assist with.
- At first we'll just do this analysis manually; if we keep the precomputation model we'll update this to subscribe to the `crates.io` RSS feed (https://crates.io/data-access)

## Walkthrough examples

Here I'll walk through the specific steps we'll do with a specific build script seen in the wild.

### Crate without a build script at all.

TODO

### Crate with a "const" build script.

TODO

### Crate with a "pure" build script.

TODO

### Crate with a "side-effectful" build script.

TODO

## Statistics

The plan here will be to write up basic classifiers and then get a percentage distribution of crates on `crates.io` for each kind.

TODO

## Notes

- `crates.io` index: `git clone https://github.com/rust-lang/crates.io-index.git`
- `cargo clone`: https://crates.io/crates/cargo-clone
