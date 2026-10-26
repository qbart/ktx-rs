# KTX for Rust language

KTX is a Rust library as pure rust port of KTX-software in C++ https://github.com/KhronosGroup/KTX-Software.

Library port focues only on two components, rest is ignored:
- libktx
- ktx cli

The only C bindings that are allowed:
- Vulkan
- OpenGL {,ES} (only behind the opt-in `opengl` feature, for the GL texture loader)

So it means that external dependencies should be ported as well or rust crates be used instead assuming they:
- are covering all required features
- are pure rust otherwise it breaks the main goal of this library port

Especially focus on full implementation of Basis Universal integration and transcoding for all types.

## Rules

1. Keep the copy of original KTX-Software in `./ref/ktx` and keep the `./ref` in gitignore
2. Follow the original API as close as possible, that includes freeing memory manually.
3. Do not use .unwrap / .expect. Library should create it's own error handling apprach (`err, ok := Operation` like in golang is preferred to simplify the API unless ktx lib assumes otherwise, however runtime exceptions are not allowed).
4. Make sure to port original tests 1:1.
5. Make sure to copy the test data like images to the library and version them in repository.
6. Maintain implementation audit logs at `./docs/spec/*.md` First write the spec there but be concise.
7. Maintain ROADMAP.md that keeps the progress about whole project.
8. Only use main branch.

## Loop iteration

1. Analyze what is missing from required features
2. Implement API partially, provide atomic changes.
3. Commit after each change, use semenatic git commits, keep the messages short, skip the annoying Co-Author commits.
4. Verify tests
5. Fix what is needed until tests are passing.
7. Push changes
8. Notify about changes done in max 5 sentence summary.
