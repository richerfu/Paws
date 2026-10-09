# openharmony-ability host snapshot

`native_ability/` is the unmodified ArkTS/C++ host from
[`richerfu/openharmony-ability`](https://github.com/richerfu/openharmony-ability/tree/2b5defdb0a016b841736470f9c32f2929380c448)
at commit `2b5defdb0a016b841736470f9c32f2929380c448`.

Arkit `06b2bb2ccf31ea2ab2f96e7f28c8dd32dab6aecd` pins its Rust Ability facade
to this revision. The published `@ohos-rs/ability` beta.2 HAR predates the
matching window focus and input lifecycle interfaces, so Paws builds this
host locally. The files/URL Rust plugins use the same Git source to share
one set of bridge traits.

When upgrading Arkit, replace this snapshot with the host from the Ability
revision in Arkit's workspace manifest, update both Rust plugin pins, and
run `ohpm install --all` before packaging. Upstream licensing is retained
in `LICENSE`.
