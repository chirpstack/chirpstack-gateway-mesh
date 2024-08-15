# ChirpStack Gateway Mesh

ChirpStack Gateway Mesh is a software component that can turn a LoRa<sup>&reg;</sup>
gateway into a relay. This makes it possible to extend the LoRa coverage, without
the need to connect each LoRa gateway to the internet.

## Documentation and binaries

Please refer to the [ChirpStack Gateway Mesh](https://www.chirpstack.io/docs/chirpstack-gateway-mesh/)
for documentation and pre-compiled binaries.

## Building from source

### Requirements

Building ChirpStack Gateway Mesh requires:

* [Nix](https://nixos.org/download.html) (recommended) and
* [Docker](https://www.docker.com/)

#### Nix

Nix is used for setting up the development environment which is used for local
development and compiling the binaries. As an alternative, you could install
these dependencies manually, please refer to `shell.nix`.

#### Docker

Docker is used by [cross-rs](https://github.com/cross-rs/cross) for cross-compiling,
as well as some of the `make` commands.

### Starting the development shell

Execute the following command to start the development shell:

```bash
nix-shell
```

### Running tests

Execute the following command to run the tests:

```bash
make test
```

### Compiling binaries

Execute the following commands to build the ChirpStack Gateway Mesh binaries and
packages:

```bash
# Only compile binaries
make build

# Compile binaries and build distributable packages
make dist
```

## License

ChirpStack Gateway Mesh is distributed under the MIT license. See also [LICENSE](https://github.com/chirpstack/chirpstack-gateway-mesh/blob/master/LICENSE).
