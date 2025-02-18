.PHONY: dist

# Compile the binaries for all targets.
build: \
	build-aarch64-unknown-linux-musl \
	build-x86_64-unknown-linux-musl \
	build-armv5te-unknown-linux-musleabi \
	build-armv7-unknown-linux-musleabihf \
	build-mips-unknown-linux-musl \
	build-mipsel-unknown-linux-musl

build-x86_64-unknown-linux-musl:
	cross build --target x86_64-unknown-linux-musl --release

build-aarch64-unknown-linux-musl:
	cross build --target aarch64-unknown-linux-musl --release

build-armv5te-unknown-linux-musleabi:
	cross build --target armv5te-unknown-linux-musleabi --release

build-armv7-unknown-linux-musleabihf:
	cross build --target armv7-unknown-linux-musleabihf --release

build-mips-unknown-linux-musl:
	# mips is a tier-3 target.
	rustup toolchain add nightly-2025-02-14-x86_64-unknown-linux-gnu
	cross +nightly-2025-02-14 build -Z build-std=panic_abort,std --target mips-unknown-linux-musl --release

build-mipsel-unknown-linux-musl:
	# mipsel is a tier-3 target.
	rustup toolchain add nightly-2025-02-14-x86_64-unknown-linux-gnu
	cross +nightly-2025-02-14 build -Z build-std=panic_abort,std --target mipsel-unknown-linux-musl --release

# Build distributable binaries for all targets.
dist: \
	dist-x86_64-unknown-linux-musl \
	dist-aarch64-unknown-linux-musl \
	dist-armv5te-unknown-linux-musleabi \
	dist-armv7-unknown-linux-musleabihf \
	dist-mips-unknown-linux-musl \
	dist-mipsel-unknown-linux-musl

dist-x86_64-unknown-linux-musl: build-x86_64-unknown-linux-musl package-x86_64-unknown-linux-musl

dist-aarch64-unknown-linux-musl: build-aarch64-unknown-linux-musl package-aarch64-unknown-linux-musl

dist-armv5te-unknown-linux-musleabi: build-armv5te-unknown-linux-musleabi package-armv5te-unknown-linux-musleabi

dist-armv7-unknown-linux-musleabihf: build-armv7-unknown-linux-musleabihf package-armv7-unknown-linux-musleabihf

dist-mips-unknown-linux-musl: build-mips-unknown-linux-musl package-mips-unknown-linux-musl

dist-mipsel-unknown-linux-musl: build-mipsel-unknown-linux-musl package-mipsel-unknown-linux-musl

# Package the compiled binaries
package-x86_64-unknown-linux-musl:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

	# .tar.gz
	tar -czvf dist/chirpstack-gateway-mesh_$(PKG_VERSION)_amd64.tar.gz -C target/x86_64-unknown-linux-musl/release chirpstack-gateway-mesh

	# .deb
	cargo deb --target x86_64-unknown-linux-musl --no-build --no-strip
	cp target/x86_64-unknown-linux-musl/debian/*.deb ./dist

package-aarch64-unknown-linux-musl:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

	# .tar.gz
	tar -czvf dist/chirpstack-gateway-mesh_$(PKG_VERSION)_arm64.tar.gz -C target/aarch64-unknown-linux-musl/release chirpstack-gateway-mesh

	# .deb
	cargo deb --target aarch64-unknown-linux-musl --no-build --no-strip
	cp target/aarch64-unknown-linux-musl/debian/*.deb ./dist


package-armv7-unknown-linux-musleabihf:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

	# .tar.gz
	tar -czvf dist/chirpstack-gateway-mesh_$(PKG_VERSION)_armv7hf.tar.gz -C target/armv7-unknown-linux-musleabihf/release chirpstack-gateway-mesh

	# .deb
	cargo deb --target armv7-unknown-linux-musleabihf --no-build --no-strip
	cp target/armv7-unknown-linux-musleabihf/debian/*.deb ./dist

package-armv5te-unknown-linux-musleabi:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

package-mips-unknown-linux-musl:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

package-mipsel-unknown-linux-musl:
	$(eval PKG_VERSION := $(shell cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version'))
	mkdir -p dist

# Update the version.
version:
	test -n "$(VERSION)"
	sed -i 's/^  version.*/  version = "$(VERSION)"/g' ./Cargo.toml
	make test
	git add .
	git commit -v -m "Bump version to $(VERSION)"
	git tag -a v$(VERSION) -m "v$(VERSION)"

# Cleanup dist.
clean:
	cargo clean
	rm -rf dist

# Run tests
test:
	cargo clippy --no-deps
	cargo test

# Enter the devshell.
devshell:
	nix-shell

# Dependencies
dev-dependencies:
	cargo install cross --git https://github.com/cross-rs/cross --rev c7dee4d008475ce1c140773cbcd6078f4b86c2aa --locked
