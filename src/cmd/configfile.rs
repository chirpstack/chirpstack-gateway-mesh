use crate::config;
use handlebars::{Handlebars, no_escape};

pub fn run() {
    let template = r#"# Logging settings.
[logging]

  # Log level.
  #
  # Valid options are:
  #   * TRACE
  #   * DEBUG
  #   * INFO
  #   * WARN
  #   * ERROR
  #   * OFF
  level="INFO"

  # Log to syslog.
  #
  # When set to true, log messages are being written to syslog instead of stdout.
  log_to_syslog=false


# Mesh configuration.
[mesh]
  # Mesh root key (AES128, HEX encoded).
  #
  # This key is used to derive the signing and encryption keys. The same key
  # must be configured on every Border and Relay gateway.
  root_key="{{ mesh.root_key }}"

  # Signing key (AES128, HEX encoded) (deprecated).
  #
  # This key is used to sign and validate each mesh packet. This key must be
  # configured on every Border / Relay gateway equally.
  #
  # Deprecation note: If set, the signing key will not be derrived from the
  # above root_key, but this key will be used.
  signing_key="{{ mesh.signing_key }}"

  # Relay ID.
  #
  # If set, this will override the Relay ID that is derived from the
  # Gateway ID provided by the Concentratord backend (using the 4 least
  # significant bytes). Example: "01020304".
  relay_id="{{ mesh.relay_id }}"

  # Border Gateway.
  #
  # If this is set to true, then the ChirpStack Gateway Mesh will consider
  # this gateway as a Border Gateway, meaning that it will unwrap relayed
  # uplinks and forward these to the proxy API, rather than relaying these.
  border_gateway={{ mesh.border_gateway }}

  # Max hop count.
  #
  # This defines the maximum number of hops a relayed payload will pass.
  max_hop_count={{ mesh.max_hop_count }}

  # Ignore direct uplinks (Border Gateway).
  #
  # If this is set to true, then direct uplinks (uplinks that are not relay
  # encapsulated) will be silently ignored. This option is especially useful
  # for testing, in which case you want to set this to true for the Border
  # Gateway.
  border_gateway_ignore_direct_uplinks={{ mesh.border_gateway_ignore_direct_uplinks }}

  # Mesh frequencies.
  #
  # The ChirpStack Gateway Mesh will randomly use one of the configured
  # frequencies when relaying uplink and downlink messages.
  frequencies=[
    {{#each mesh.frequencies}}
    {{this}},
    {{/each}}
  ]

  # TX Power (EIRP).
  #
  # The TX Power in EIRP used when relaying uplink and downlink messages.
  tx_power={{ mesh.tx_power }}

  # Data-rate properties.
  #
  # The data-rate properties when relaying uplink and downlink messages.
  [mesh.data_rate]
  
    # Modulation.
    #
    # Valid options are: LORA, FSK
    modulation="{{ mesh.data_rate.modulation }}"

    # Spreading-factor (LoRa).
    spreading_factor={{ mesh.data_rate.spreading_factor }}

    # Bandwidth (LoRa).
    bandwidth={{ mesh.data_rate.bandwidth }}

    # Code-rate (LoRa).
    code_rate="{{ mesh.data_rate.code_rate }}"

    # Bitrate (FSK).
    bitrate={{ mesh.data_rate.bitrate }}


  # Proxy API configuration.
  #
  # If the gateway is configured to operate as Border Gateway. It
  # will unwrap relayed uplink frames, and will wrap downlink payloads that
  # must be relayed. In this case the ChirpStack MQTT Forwarder must be
  # configured to use the proxy API instead of the Concentratord API.
  #
  # Payloads of devices that are under the direct coverage of this gateway
  # are transparently proxied between the ChirpStack MQTT Forwarder and
  # ChirpStack Concentratord.
  #
  # This configuration is only used when the border_gateway option is set
  # to true.
  [mesh.proxy_api]

    # Event PUB socket bind.
    event_bind="{{ mesh.proxy_api.event_bind }}"

    # Command REP socket bind.
    command_bind="{{ mesh.proxy_api.command_bind }}"

  # Filters.
  [mesh.filters]

    # DevAddr prefix filters.
    #
    # Example configuration:
    # dev_addr_prefixes=["0000ff00/24"]
    #
    # The above filter means that the 24MSB of 0000ff00 will be used to
    # filter DevAddrs. Uplinks with DevAddrs that do not match any of the
    # configured filters will not be forwarded. Leaving this option empty
    # disables filtering on DevAddr.
    dev_addr_prefixes=[
      {{#each mesh.filters.dev_addr_prefixes}}
      "{{this}}",
      {{/each}}
    ]

    # JoinEUI prefix filters.
    #
    # Example configuration:
    # join_eui_prefixes=["0000ff0000000000/24"]
    #
    # The above filter means that the 24MSB of 0000ff0000000000 will be used
    # to filter JoinEUIs. Uplinks with JoinEUIs that do not match any of the
    # configured filters will not be forwarded. Leaving this option empty
    # disables filtering on JoinEUI.
    join_eui_prefixes=[
      {{#each mesh.filters.join_eui_prefixes}}
      "{{this}}",
      {{/each}}
    ]

    # LoRaWAN only.
    lorawan_only={{mesh.filters.lorawan_only}}


# Backend configuration.
[backend]

  # ChirpStack Concentratord configuration (end-device communication).
  [backend.concentratord]

    # Event API URL.
    event_url="{{ backend.concentratord.event_url }}"

    # Command API URL.
    command_url="{{ backend.concentratord.command_url }}"


  # ChirpStack Concentratord configuration (mesh communication).
  #
  # While not required, this configuration makes it possible to use a different
  # Concentratord instance for the mesh communication. E.g. this
  # makes it possible to use ISM2400 for mesh communication and EU868 for
  # communication with the end-devices.
  [backend.mesh_concentratord]

    # Event API URL.
    event_url="{{ backend.mesh_concentratord.event_url }}"

    # Command API URL.
    command_url="{{ backend.mesh_concentratord.command_url }}"


# Events configuration (Relay only).
[events]

  # Heartbeat interval (Relay Gateway only).
  #
  # This defines the interval in which a Relay Gateway (border_gateway=false)
  # will emit heartbeat messages.
  heartbeat_interval="{{ events.heartbeat_interval }}"

  # Commands.
  #
  # This configures for each event type the command that must be executed. The
  # stdout of the command will be used as event payload. Example:
  #
  #   128 = ["/path/to/command", "arg1", "arg2"]
  #
  [events.commands]

    {{#each events.commands}}
    {{@key}} = [{{#each this}}"{{this}}", {{/each}}]
    {{/each}}

  # Event sets (can be repeated).
  #
  # This configures sets of events that will be periodically sent by the
  # relay. Example:
  #
  #  [[events.sets]]
  #    interval = "5min"
  #    events = [128, 129, 130]
  #
  {{#each events.sets}}
  [[events.sets]]
    interval = "{{this.interval}}"
    events = [{{#each this.events}}{{this}}, {{/each}}]
  {{/each}}


# Commands configuration (Relay only).
[commands]

  # Commands.
  #
  # On receiving a command, the Gateway Mesh will execute the command matching
  # the command type (128 - 255 is for proprietary commands). The payload will
  # be provided to the command using stdin. The returned stdout will be sent
  # back as event (using the same type). Example:
  #
  #   "129" = ["/path/to/command", "arg1", "arg2"]
  #
  [commands.commands]

    {{#each commands.commands}}
    {{@key}} = [{{#each this}}"{{this}}", {{/each}}]
    {{/each}}
"#;

    let conf = config::get();
    let mut reg = Handlebars::new();
    reg.register_escape_fn(no_escape);
    println!(
        "{}",
        reg.render_template(template, &(*conf))
            .expect("Render configfile error")
    );
}
