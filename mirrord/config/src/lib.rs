#![feature(slice_concat_trait)]
#![feature(iterator_try_collect)]
#![feature(let_chains)]
#![warn(clippy::indexing_slicing)]
#![deny(unused_crate_dependencies)]

//! <!--${internal}-->
//! To generate the `mirrord-schema.json` file see
//! `tests::check_schema_file_exists_and_is_valid_or_create_it`.
//!
//! Remember to re-generate the `mirrord-schema.json` if you make **ANY** changes to this lib,
//! including if you only made documentation changes.
pub mod agent;
pub mod config;
pub mod container;
pub mod experimental;
pub mod external_proxy;
pub mod feature;
pub mod internal_proxy;
pub mod target;
pub mod util;

use std::{
    collections::HashMap,
    ops::Not,
    path::{Path, PathBuf},
    time::SystemTime,
};

use base64::prelude::*;
use config::{ConfigContext, ConfigError, MirrordConfig};
use experimental::ExperimentalConfig;
use feature::{env::mapper::EnvVarsRemapper, network::outgoing::OutgoingFilterConfig};
use mirrord_analytics::CollectAnalytics;
use mirrord_config_derive::MirrordConfig;
use rand::distr::{Alphanumeric, SampleString};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use target::Target;
use tera::Tera;
use tracing::warn;

use crate::{
    agent::AgentConfig,
    config::source::MirrordConfigSource,
    container::ContainerConfig,
    external_proxy::ExternalProxyConfig,
    feature::{
        fs::{READONLY_FILE_BUFFER_HARD_LIMIT, READONLY_FILE_BUFFER_WARN_LIMIT},
        FeatureConfig,
    },
    internal_proxy::InternalProxyConfig,
    target::TargetConfig,
    util::VecOrSingle,
};

/// Environment variable we use to pass the internal proxy address to the layer.
pub const MIRRORD_LAYER_INTPROXY_ADDR: &str = "MIRRORD_LAYER_INTPROXY_ADDR";

/// mirrord allows for a high degree of customization when it comes to which features you want to
/// enable, and how they should function.
///
/// All of the configuration fields have a default value, so a minimal configuration would be no
/// configuration at all.
///
/// The configuration supports templating using the [Tera](https://keats.github.io/tera/docs/) template engine.
/// Currently we don't provide additional values to the context, if you have anything you want us to
/// provide please let us know.
///
/// To use a configuration file in the CLI, use the `-f <CONFIG_PATH>` flag.
/// Or if using VSCode Extension or JetBrains plugin, simply create a `.mirrord/mirrord.json` file
/// or use the UI.
///
/// To help you get started, here are examples of a basic configuration file, and a complete
/// configuration file containing all fields.
///
/// ### Basic `config.json` {#root-basic}
///
/// ```json
/// {
///   "target": "pod/bear-pod",
///   "feature": {
///     "env": true,
///     "fs": "read",
///     "network": true
///   }
/// }
/// ```
///
/// ### Basic `config.json` with templating {#root-basic-templating}
///
/// ```json
/// {
///   "target": "{{ get_env(name="TARGET", default="pod/fallback") }}",
///   "feature": {
///     "env": true,
///     "fs": "read",
///     "network": true
///   }
/// }
/// ```
///
/// ### Complete `config.json` {#root-complete}
///
///  Don't use this example as a starting point, it's just here to show you all the available
///  options.
/// ```json
/// {
///   "accept_invalid_certificates": false,
///   "skip_processes": "ide-debugger",
///   "target": {
///     "path": "pod/bear-pod",
///     "namespace": "default"
///   },
///   "connect_tcp": null,
///   "agent": {
///     "log_level": "info",
///     "json_log": false,
///     "labels": { "user": "meow" },
///     "annotations": { "cats.io/inject": "enabled" },
///     "namespace": "default",
///     "image": "ghcr.io/metalbear-co/mirrord:latest",
///     "image_pull_policy": "IfNotPresent",
///     "image_pull_secrets": [ { "secret-key": "secret" } ],
///     "ttl": 30,
///     "ephemeral": false,
///     "communication_timeout": 30,
///     "startup_timeout": 360,
///     "network_interface": "eth0",
///     "flush_connections": true,
///     "metrics": "0.0.0.0:9000",
///   },
///   "feature": {
///     "env": {
///       "include": "DATABASE_USER;PUBLIC_ENV",
///       "exclude": "DATABASE_PASSWORD;SECRET_ENV",
///       "override": {
///         "DATABASE_CONNECTION": "db://localhost:7777/my-db",
///         "LOCAL_BEAR": "panda"
///       },
///       "mapping": {
///         ".+_TIMEOUT": "1000"
///       }
///     },
///     "fs": {
///       "mode": "write",
///       "read_write": ".+\\.json" ,
///       "read_only": [ ".+\\.yaml", ".+important-file\\.txt" ],
///       "local": [ ".+\\.js", ".+\\.mjs" ]
///     },
///     "network": {
///       "incoming": {
///         "mode": "steal",
///         "http_filter": {
///           "header_filter": "host: api\\..+"
///         },
///         "port_mapping": [[ 7777, 8888 ]],
///         "ignore_localhost": false,
///         "ignore_ports": [9999, 10000]
///       },
///       "outgoing": {
///         "tcp": true,
///         "udp": true,
///         "filter": {
///           "local": ["tcp://1.1.1.0/24:1337", "1.1.5.0/24", "google.com", ":53"]
///         },
///         "ignore_localhost": false,
///         "unix_streams": "bear.+"
///       },
///       "dns": {
///         "enabled": true,
///         "filter": {
///           "local": ["1.1.1.0/24:1337", "1.1.5.0/24", "google.com"]
///         }
///       }
///     },
///     "copy_target": {
///       "scale_down": false
///     }
///   },
///   "operator": true,
///   "kubeconfig": "~/.kube/config",
///   "sip_binaries": "bash",
///   "telemetry": true,
///   "kube_context": "my-cluster"
/// }
/// ```
///
/// # Options {#root-options}
#[derive(MirrordConfig, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[config(map_to = "LayerFileConfig", derive = "JsonSchema")]
#[cfg_attr(test, config(derive = "PartialEq"))]
pub struct LayerConfig {
    /// ## accept_invalid_certificates {#root-accept_invalid_certificates}
    ///
    /// Controls whether or not mirrord accepts invalid TLS certificates (e.g. self-signed
    /// certificates).
    ///
    /// If not provided, mirrord will use value from the kubeconfig.
    #[config(env = "MIRRORD_ACCEPT_INVALID_CERTIFICATES")]
    pub accept_invalid_certificates: Option<bool>,

    /// ## skip_processes {#root-skip_processes}
    ///
    /// Allows mirrord to skip unwanted processes.
    ///
    /// Useful when process A spawns process B, and the user wants mirrord to operate only on
    /// process B.
    /// Accepts a single value, or an array of values.
    ///
    ///```json
    /// {
    ///  "skip_processes": ["bash", "node"]
    /// }
    /// ```
    #[config(env = "MIRRORD_SKIP_PROCESSES")]
    pub skip_processes: Option<VecOrSingle<String>>,

    /// ## skip_build_tools {#root-skip_build_tools}
    ///
    /// Allows mirrord to skip build tools. Useful when running command lines that build and run
    /// the application in a single command.
    ///
    /// Defaults to `true`.
    ///
    /// Build-Tools: `["as", "cc", "ld", "go", "air", "asm", "cc1", "cgo", "dlv", "gcc", "git",
    /// "link", "math", "cargo", "hpack", "rustc", "compile", "collect2", "cargo-watch",
    /// "debugserver"]`
    #[config(env = "MIRRORD_SKIP_BUILD_TOOLS", default = true)]
    pub skip_build_tools: bool,

    /// ## skip_extra_build_tools {#root-skip_build_tools}
    ///
    /// Allows mirrord to skip the specified build tools. Useful when running command lines that
    /// build and run the application in a single command.
    ///
    /// Must also enable [`skip_build_tools`](#root-skip_build_tools) for this to take an effect.
    ///
    /// It's similar to [`skip_processes`](#root-skip_processes), except that here it also skips
    /// SIP patching.
    ///
    /// Accepts a single value, or an array of values.
    ///
    ///```json
    /// {
    ///  "skip_extra_build_tools": ["bash", "node"]
    /// }
    /// ```
    #[config(env = "MIRRORD_SKIP_EXTRA_BUILD_TOOLS")]
    pub skip_extra_build_tools: Option<VecOrSingle<String>>,

    /// ## operator {#root-operator}
    ///
    /// Whether mirrord should use the operator.
    /// If not set, mirrord will first attempt to use the operator, but continue without it in case
    /// of failure.
    #[config(env = "MIRRORD_OPERATOR_ENABLE")]
    pub operator: Option<bool>,

    /// ## profile {#root-profile}
    ///
    /// Name of the mirrord profile to use.
    ///
    /// To select a cluster-wide profile
    ///
    /// ```json
    /// {
    ///   "profile": "my-profile-name"
    /// }
    /// ```
    ///
    /// To select a namespaced profile
    ///
    /// ```json
    /// {
    ///   "profile": "my-namespace/my-profile-name"
    /// }
    /// ```
    pub profile: Option<String>,

    /// ## kubeconfig {#root-kubeconfig}
    ///
    /// Path to a kubeconfig file, if not specified, will use `KUBECONFIG`, or `~/.kube/config`, or
    /// the in-cluster config.
    ///
    /// ```json
    /// {
    ///   "kubeconfig": "~/bear/kube-config"
    /// }
    /// ```
    #[config(env = "MIRRORD_KUBECONFIG")]
    pub kubeconfig: Option<String>,

    /// ## sip_binaries {#root-sip_binaries}
    ///
    /// Binaries to patch (macOS SIP).
    ///
    /// Use this when mirrord isn't loaded to protected binaries that weren't automatically
    /// patched.
    ///
    /// Runs `endswith` on the binary path (so `bash` would apply to any binary ending with `bash`
    /// while `/usr/bin/bash` would apply only for that binary).
    ///
    /// ```json
    /// {
    ///   "sip_binaries": ["bash", "python"]
    /// }
    /// ```
    pub sip_binaries: Option<VecOrSingle<String>>,

    /// ## target {#root-target}
    #[config(nested)]
    pub target: TargetConfig,

    /// ## agent {#root-agent}
    #[config(nested)]
    pub agent: AgentConfig,

    /// ## container {#root-container}
    #[config(nested, unstable)]
    pub container: ContainerConfig,

    /// ## feature {#root-feature}
    #[config(nested)]
    pub feature: FeatureConfig,

    /// ## telemetry {#root-telemetry}
    /// Controls whether or not mirrord sends telemetry data to MetalBear cloud.
    /// Telemetry sent doesn't contain personal identifiers or any data that
    /// should be considered sensitive. It is used to improve the product.
    /// [For more information](https://github.com/metalbear-co/mirrord/blob/main/TELEMETRY.md)
    #[config(env = "MIRRORD_TELEMETRY", default = true)]
    pub telemetry: bool,

    /// ## kube_context {#root-kube_context}
    ///
    /// Kube context to use from the kubeconfig file.
    /// Will use current context if not specified.
    ///
    /// ```json
    /// {
    ///   "kube_context": "mycluster"
    /// }
    /// ```
    #[config(env = "MIRRORD_KUBE_CONTEXT")]
    pub kube_context: Option<String>,

    /// ## internal_proxy {#root-internal_proxy}
    #[config(nested)]
    pub internal_proxy: InternalProxyConfig,

    /// ## external_proxy {#root-external_proxy}
    #[config(nested)]
    pub external_proxy: ExternalProxyConfig,

    /// ## use_proxy {#root-use_proxy}
    ///
    /// When disabled, mirrord will remove `HTTP[S]_PROXY` env variables before
    /// doing any network requests. This is useful when the system sets a proxy
    /// but you don't want mirrord to use it.
    /// This also applies to the mirrord process (as it just removes the env).
    /// If the remote pod sets this env, the mirrord process will still use it.
    #[config(env = "MIRRORD_PROXY", default = true)]
    pub use_proxy: bool,

    /// ## experimental {#root-experimental}
    #[config(nested)]
    pub experimental: ExperimentalConfig,

    /// ## skip_sip {#root-skip_sip}
    ///
    /// Allows mirrord to skip patching (macOS SIP) unwanted processes.
    ///
    /// When patching is skipped, mirrord will no longer be able to load into
    /// the process and its child processes.
    ///
    /// Defaults to `{ "skip_sip": "git" }`
    ///
    /// When specified, the given value will replace the default list rather than
    /// being added to.
    #[config(env = "MIRRORD_SKIP_SIP", default = VecOrSingle::Single("git".to_string()))]
    pub skip_sip: VecOrSingle<String>,
}

impl LayerConfig {
    /// Env variable where we set the path to the [`LayerConfig`].
    ///
    /// Used by the extensions and when we transform CLI arguments into environment variables.
    ///
    /// Used in [`LayerConfig::resolve`].
    pub const FILE_PATH_ENV: &str = "MIRRORD_CONFIG_FILE";

    /// Env variable where we store encoded resolved config.
    ///
    /// mirrord CLI children should not [`LayerConfig::resolve`] the configuration again,
    /// instead they should use the already resolved config.
    ///
    /// See [`LayerConfig::encode`] and [`LayerConfig::decode`].
    pub const RESOLVED_CONFIG_ENV: &str = "MIRRORD_RESOLVED_CONFIG";

    /// Decodes an encoded [`LayerConfig`].
    ///
    /// You can encode the config with [`LayerConfig::encode`].
    pub fn decode(encoded_value: &str) -> Result<Self, ConfigError> {
        let decoded = BASE64_STANDARD
            .decode(encoded_value)
            .map_err(|error| ConfigError::DecodeError(error.to_string()))?;
        let deserialized = serde_json::from_slice(&decoded)
            .map_err(|error| ConfigError::DecodeError(error.to_string()))?;

        Ok(deserialized)
    }

    /// Encodes this config to a string.
    ///
    /// You can decode the config with [`LayerConfig::decode`].
    pub fn encode(&self) -> Result<String, ConfigError> {
        let serialized = serde_json::to_string(self)
            .map_err(|error| ConfigError::EncodeError(error.to_string()))?;
        let encoded = BASE64_STANDARD.encode(serialized);

        Ok(encoded)
    }

    /// Resolves the config from the environment variables.
    ///
    /// On success, returns the config and a [`ConfigContext`] that holds warnings.
    /// To be used from CLI entry points to resolve user config and print warnings.
    ///
    /// This function **does not** use [`LayerConfig::RESOLVED_CONFIG_ENV`] nor
    /// [`LayerConfig::decode`]. It resolves the config from scratch.
    pub fn resolve(context: &mut ConfigContext) -> Result<Self, ConfigError> {
        let mut config = if let Ok(path) = context.get_env(Self::FILE_PATH_ENV) {
            LayerFileConfig::from_path(path)?.generate_config(context)
        } else {
            LayerFileConfig::default().generate_config(context)
        }?;

        // OSS-specific adjustment
        //
        // `agent.passthrough_mirroring` is enabled by default in OSS.
        config.agent.passthrough_mirroring = config.agent.passthrough_mirroring.or(Some(true));

        Ok(config)
    }

    /// Verifies that there are no conflicting settings in this config.
    ///
    /// Fills the given [`ConfigContext`] with warnings.
    pub fn verify(&self, context: &mut ConfigContext) -> Result<(), ConfigError> {
        if self.agent.ephemeral && self.agent.namespace.is_some() {
            context.add_warning(
                "Agent namespace is ignored when using an ephemeral container for the agent."
                    .to_string(),
            );
        }

        if matches!(
            self.feature.network.outgoing.filter,
            Some(OutgoingFilterConfig::Remote(_))
        ) && !self.feature.network.dns.enabled
        {
            context.add_warning(
                "The mirrord outgoing traffic filter includes host names to be connected remotely, \
                but the remote DNS feature is disabled, so the addresses of these hosts will be \
                resolved locally. Consider enabling the remote DNS resolution feature.".to_string(),
            );
        }

        let http_filter = &self.feature.network.incoming.http_filter;
        let used_filters = [
            http_filter.path_filter.is_some(),
            http_filter.header_filter.is_some(),
            http_filter.all_of.is_some(),
            http_filter.any_of.is_some(),
        ]
        .into_iter()
        .filter(|used| *used)
        .count();
        if used_filters > 1 {
            Err(ConfigError::Conflict(
                "Cannot use multiple types of HTTP filter at the same time, use 'any_of' or 'all_of' to combine filters".to_string(),
            ))?
        }

        if [http_filter.all_of.as_ref(), http_filter.any_of.as_ref()]
            .into_iter()
            .flatten()
            .any(Vec::is_empty)
        {
            Err(ConfigError::Conflict(
                "Composite HTTP filter cannot be empty".to_string(),
            ))?;
        }

        if !self.feature.network.incoming.ignore_ports.is_empty()
            && self.feature.network.incoming.ports.is_some()
        {
            Err(ConfigError::Conflict(
                "Cannot use both `incoming.ignore_ports` and `incoming.ports` at the same time"
                    .to_string(),
            ))?
        }

        if let (Some(unfiltered_ports), Some(filtered_ports)) = (
            self.feature.network.incoming.ports.as_ref(),
            self.feature
                .network
                .incoming
                .http_filter
                .get_filtered_ports(),
        ) {
            let intersection = filtered_ports
                .iter()
                .copied()
                .filter(|port| unfiltered_ports.contains(port))
                .collect::<Vec<_>>();
            if intersection.is_empty().not() {
                Err(ConfigError::Conflict(format!(
                    "Ports {intersection:?} are present in both `feature.network.incoming.ports` and \
                    `feature.network.incoming.http_filter.ports`. These lists must remain disjoint. \
                    If you want traffic to a port to be filtered, \
                    include it only in `feature.network.incoming.http_filter.ports`. \
                    To steal all the traffic from that port without filtering, \
                    include it only in `feature.network.incoming.ports`."
                )))?
            }
        }

        match (
            &self.feature.network.incoming.https_delivery,
            &self.feature.network.incoming.tls_delivery,
        ) {
            (Some(..), Some(..)) => {
                return Err(ConfigError::Conflict(
                    "Cannot use both `feature.network.incoming.https_delivery` \
                    and `feature.network.incoming.tls_delivery` at the same time"
                        .to_string(),
                ));
            }
            (Some(config), ..) => {
                context.add_warning(
                    "`feature.network.incoming.https_delivery` is deprecated, \
                    use `feature.network.incoming.tls_delivery` instead."
                        .into(),
                );
                config.verify(context)?
            }
            (.., Some(config)) => config.verify(context)?,
            (None, None) => {}
        }

        if !self.feature.copy_target.enabled
            && self
                .target
                .path
                .as_ref()
                .map(Target::requires_copy)
                .unwrap_or_default()
        {
            Err(ConfigError::TargetJobWithoutCopyTarget)?
        }

        let is_targetless = match self.target.path.as_ref() {
            Some(Target::Targetless) => true,
            None => context.is_empty_target_final().not(),
            _ => false,
        };

        if is_targetless {
            if self.feature.network.incoming.is_steal() {
                Err(ConfigError::Conflict("Steal mode is not compatible with a targetless agent, please either disable this option or specify a target.".into()))?
            }

            if self.agent.ephemeral {
                Err(ConfigError::Conflict(
                    "Using an ephemeral container for the agent is not \
                         compatible with a targetless agent, please either disable this option or \
                        specify a target."
                        .into(),
                ))?
            }

            if self.agent.namespace.is_some() {
                context.add_warning(
                    "Agent namespace is ignored in targetless runs. \
                    To specify a namespace for a targetless run, use target namespace."
                        .into(),
                );
            }
        }

        if self.feature.copy_target.enabled {
            if self.operator == Some(false) {
                return Err(ConfigError::Conflict(
                    "The copy target feature requires a mirrord operator, \
                   please either disable this option or use the operator."
                        .into(),
                ));
            }

            // Target may also be set later in the UI.
            if is_targetless {
                return Err(ConfigError::Conflict(
                    "The copy target feature is not compatible with a targetless agent, \
                    please either disable this option or specify a target."
                        .into(),
                ));
            }

            if matches!(self.target.path, Some(Target::Service(..))) {
                return Err(ConfigError::Conflict(
                    "The copy target feature is not yet supported with service targets, \
                    please either disable this option or specify an exact workload covered by this service."
                        .into()
                ));
            }

            if !self.feature.network.incoming.is_steal() {
                context.add_warning(
                    "Using copy target feature without steal mode \
                    may result in unreturned responses in cluster \
                    because the underlying app instance is not copied \
                    and therefore not running in the copied pod"
                        .into(),
                );
            }
        }

        // operator is disabled, but target requires it.
        if self
            .target
            .path
            .as_ref()
            .is_some_and(Target::requires_operator)
            && !self.operator.unwrap_or(true)
        {
            return Err(ConfigError::TargetRequiresOperator);
        }

        if self
            .feature
            .network
            .incoming
            .port_mapping
            .iter()
            .any(|(to, from)| to == from)
        {
            context.add_warning(
                "The feature.network.incoming.port_mapping mirrord configuration field \
                contains a mapping of a local port to the same remote port. \
                A mapping is only necessary when the local application is listening on \
                a different port than the remote one."
                    .into(),
            );
        }

        // Env vars
        if self.feature.env.exclude.is_some() && self.feature.env.include.is_some() {
            return Err(ConfigError::Conflict(
                "cannot use both `include` and `exclude` filters for environment variables"
                    .to_string(),
            ));
        }

        if let Some(env_vars_mapping) = self.feature.env.mapping.clone() {
            EnvVarsRemapper::new(env_vars_mapping, HashMap::new())?;
        }

        self.feature.network.dns.verify(context)?;
        self.feature.network.outgoing.verify(context)?;
        self.feature.split_queues.verify(context)?;

        if self.experimental.readlink {
            context.add_warning(
                "experimental.readlink config has been deprecated, and `readlink` is now\
                    enabled by default! You may remove it from your config."
                    .into(),
            );
        }

        if self.experimental.readonly_file_buffer.is_some() {
            return Err(ConfigError::Conflict(
                "cannot use experimental.readonly_file_buffer, as it has been moved. Use feature.fs.readonly_file_buffer instead."
                    .to_string(),
            ));
        }

        if self.feature.fs.readonly_file_buffer > READONLY_FILE_BUFFER_HARD_LIMIT {
            return Err(ConfigError::InvalidValue {
                name: "feature.fs.readonly_file_buffer",
                provided: self.feature.fs.readonly_file_buffer.to_string(),
                error: format!(
                    "the value of feature.fs.readonly_file_buffer must be {} Megabytes or less.",
                    READONLY_FILE_BUFFER_HARD_LIMIT / 1024 / 1024
                )
                .into(),
            });
        } else if self.feature.fs.readonly_file_buffer > READONLY_FILE_BUFFER_WARN_LIMIT {
            context.add_warning(format!(
                "The value of feature.fs.readonly_file_buffer is more than {} Megabyte. \
                     Large values may increase the risk of timeouts.",
                READONLY_FILE_BUFFER_WARN_LIMIT / 1024 / 1024,
            ));
        }

        if let (Some(profile), true) = (&self.profile, context.has_warnings()) {
            // It might be that the user config is fine,
            // but the mirrord profile introduced changes that triggered the warnings.
            context.add_warning(format!(
                "Config verification was done after applying mirrord profile `{profile}`. \
                You can inspect the profile with `kubectl get mirrordclusterprofile {profile} -o yaml`.",
            ));
        }

        if self.feature.copy_target.enabled
            && self.feature.network.incoming.http_filter.is_filter_set()
        {
            context.add_warning(
                "copy target is enabled and http filter is set, this means that all \
            unmatched HTTP requests are discarded"
                    .to_string(),
            );
        }

        Ok(())
    }
}

impl CollectAnalytics for &LayerConfig {
    fn collect_analytics(&self, analytics: &mut mirrord_analytics::Analytics) {
        if let Some(value) = self.accept_invalid_certificates {
            analytics.add("accept_invalid_certificates", value);
        };
        analytics.add("use_kubeconfig", self.kubeconfig.is_some());
        analytics.add("use_profile", self.profile.is_some());
        (&self.target).collect_analytics(analytics);
        (&self.agent).collect_analytics(analytics);
        (&self.feature).collect_analytics(analytics);
        (&self.experimental).collect_analytics(analytics);
    }
}

impl LayerFileConfig {
    pub fn from_path<P>(path: P) -> Result<Self, ConfigError>
    where
        P: AsRef<Path>,
    {
        let mut template_engine = Tera::default();
        template_engine.add_template_file(path.as_ref(), Some("main"))?;
        let rendered = template_engine.render("main", &tera::Context::new())?;

        match path.as_ref().extension().and_then(|os_val| os_val.to_str()) {
            // No Extension? assume json
            Some("json") | None => Ok(serde_json::from_str::<Self>(&rendered)?),
            Some("toml") => Ok(toml::from_str::<Self>(&rendered)?),
            Some("yaml" | "yml") => Ok(serde_yaml::from_str::<Self>(&rendered)?),
            _ => Err(ConfigError::UnsupportedFormat),
        }
    }
}

/// Returns a default randomized path for proxy logs.
///
/// `prefix` can be passed to distinguish between intproxy and extproxy logs.
fn default_proxy_logfile_path(prefix: &str) -> PathBuf {
    let random_name: String = Alphanumeric.sample_string(&mut rand::rng(), 7);
    let timestamp = SystemTime::UNIX_EPOCH
        .elapsed()
        .expect("system time should not be earlier than UNIX EPOCH")
        .as_secs();

    let mut path = std::env::temp_dir();
    path.push(format!("{prefix}-{timestamp}-{random_name}.log"));
    path
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Write},
    };

    use rstest::*;
    use schemars::schema::RootSchema;

    use super::*;
    use crate::{
        agent::AgentFileConfig,
        feature::{
            fs::{FsModeConfig, FsUserConfig},
            network::{
                incoming::{IncomingAdvancedFileConfig, IncomingFileConfig, IncomingMode},
                outgoing::OutgoingFileConfig,
                NetworkFileConfig,
            },
            FeatureFileConfig,
        },
        target::{Target, TargetFileConfig},
        util::ToggleableConfig,
    };

    #[derive(Debug)]
    enum ConfigType {
        Json,
        Toml,
        Yaml,
    }

    impl ConfigType {
        fn empty(&self) -> &'static str {
            match self {
                ConfigType::Json => "{}",
                ConfigType::Toml => "",
                ConfigType::Yaml => "",
            }
        }

        fn issue_2647(&self) -> &'static str {
            match self {
                ConfigType::Json => {
                    r#"
                    {
                        "feature": {
                            "network": {
                                "incoming": "steal"
                            }
                        }
                    }
                    "#
                }
                ConfigType::Toml => {
                    r#"
                    [feature.network]
                    incoming = "steal"
                    "#
                }
                ConfigType::Yaml => {
                    r#"
                    feature:
                        network:
                            incoming: steal
                    "#
                }
            }
        }

        fn full(&self) -> &'static str {
            match self {
                ConfigType::Json => {
                    r#"
                    {
                        "accept_invalid_certificates": false,
                        "target": {
                            "path": "pod/test-service-abcdefg-abcd",
                            "namespace": "default"
                        },
                        "agent": {
                            "log_level": "info",
                            "json_log": false,
                            "namespace": "default",
                            "image": "",
                            "image_pull_policy": "",
                            "image_pull_secrets": [{"name": "testsecret"}],
                            "ttl": 60,
                            "ephemeral": false,
                            "flush_connections": false
                        },
                        "feature": {
                            "env": true,
                            "fs": "write",
                            "network": {
                                "dns": false,
                                "incoming": {
                                    "mode": "mirror"
                                },
                                "outgoing": {
                                    "tcp": true,
                                    "udp": false
                                }
                            }
                        }
                    }
                    "#
                }
                ConfigType::Toml => {
                    r#"
                    accept_invalid_certificates = false

                    [target]
                    path = "pod/test-service-abcdefg-abcd"
                    namespace = "default"

                    [agent]
                    log_level = "info"
                    json_log = false
                    namespace = "default"
                    image = ""
                    image_pull_policy = ""
                    image_pull_secrets = [{name = "testsecret"}]
                    ttl = 60
                    ephemeral = false
                    flush_connections = false

                    [feature]
                    env = true
                    fs = "write"

                    [feature.network]
                    dns = false

                    [feature.network.incoming]
                    mode = "mirror"

                    [feature.network.outgoing]
                    tcp = true
                    udp = false
                    "#
                }
                ConfigType::Yaml => {
                    r#"
                    accept_invalid_certificates: false
                    target:
                        path: "pod/test-service-abcdefg-abcd"
                        namespace: "default"

                    agent:
                        log_level: "info"
                        json_log: false
                        namespace: "default"
                        image: ""
                        image_pull_policy: ""
                        image_pull_secrets:
                            - name: "testsecret"
                        ttl: 60
                        ephemeral: false
                        flush_connections: false

                    feature:
                        env: true
                        fs: "write"
                        network:
                            dns: false
                            incoming:
                                mode: "mirror"
                            outgoing:
                                tcp: true
                                udp: false
                    "#
                }
            }
        }

        fn parse(&self, value: &str) -> LayerFileConfig {
            match self {
                ConfigType::Json => {
                    serde_json::from_str(value).unwrap_or_else(|err| panic!("{err:?}"))
                }
                ConfigType::Toml => toml::from_str(value).unwrap_or_else(|err| panic!("{err:?}")),
                ConfigType::Yaml => {
                    serde_yaml::from_str(value).unwrap_or_else(|err| panic!("{err:?}"))
                }
            }
        }
    }

    #[rstest]
    fn empty(
        #[values(ConfigType::Json, ConfigType::Toml, ConfigType::Yaml)] config_type: ConfigType,
    ) {
        let input = config_type.empty();

        let config = config_type.parse(input);

        assert_eq!(config, LayerFileConfig::default());
    }

    #[rstest]
    fn issue_2647(
        #[values(ConfigType::Json, ConfigType::Toml, ConfigType::Yaml)] config_type: ConfigType,
    ) {
        let input = config_type.issue_2647();
        let config = config_type.parse(input);

        let expect = LayerFileConfig {
            feature: Some(FeatureFileConfig {
                network: Some(ToggleableConfig::Config(NetworkFileConfig {
                    incoming: Some(ToggleableConfig::Config(IncomingFileConfig::Simple(Some(
                        IncomingMode::Steal,
                    )))),
                    ..Default::default()
                })),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(config, expect);
    }

    #[rstest]
    fn full(
        #[values(ConfigType::Json, ConfigType::Toml, ConfigType::Yaml)] config_type: ConfigType,
    ) {
        use crate::{
            agent::{AgentImageFileConfig, AgentPullSecret},
            target::pod::PodTarget,
        };

        let input = config_type.full();

        let config = config_type.parse(input);

        let expect = LayerFileConfig {
            accept_invalid_certificates: Some(false),
            kubeconfig: None,
            telemetry: None,
            target: Some(TargetFileConfig::Advanced {
                path: Some(Target::Pod(PodTarget {
                    pod: "test-service-abcdefg-abcd".to_owned(),
                    container: None,
                })),
                namespace: Some("default".to_owned()),
            }),
            skip_processes: None,
            skip_extra_build_tools: None,
            skip_build_tools: None,
            agent: Some(AgentFileConfig {
                privileged: None,
                log_level: Some("info".to_owned()),
                json_log: Some(false),
                namespace: Some("default".to_owned()),
                image: Some(AgentImageFileConfig::Simple(Some("".to_owned()))),
                image_pull_policy: Some("".to_owned()),
                image_pull_secrets: Some(vec![AgentPullSecret {
                    name: "testsecret".to_owned(),
                }]),
                ttl: Some(60),
                ephemeral: Some(false),
                communication_timeout: None,
                startup_timeout: None,
                network_interface: None,
                flush_connections: Some(false),
                disabled_capabilities: None,
                tolerations: None,
                check_out_of_pods: None,
                resources: None,
                nftables: None,
                ..Default::default()
            }),
            feature: Some(FeatureFileConfig {
                env: ToggleableConfig::Enabled(true).into(),
                fs: ToggleableConfig::Config(FsUserConfig::Simple(FsModeConfig::Write)).into(),
                network: Some(ToggleableConfig::Config(NetworkFileConfig {
                    dns: Some(ToggleableConfig::Enabled(false)),
                    incoming: Some(ToggleableConfig::Config(IncomingFileConfig::Advanced(
                        Box::new(IncomingAdvancedFileConfig {
                            mode: Some(IncomingMode::Mirror),
                            http_filter: None,
                            port_mapping: None,
                            ignore_localhost: None,
                            ignore_ports: None,
                            listen_ports: None,
                            on_concurrent_steal: None,
                            ports: None,
                            https_delivery: Default::default(),
                            tls_delivery: Default::default(),
                        }),
                    ))),
                    outgoing: Some(ToggleableConfig::Config(OutgoingFileConfig {
                        tcp: Some(true),
                        udp: Some(false),
                        ..Default::default()
                    })),
                    ipv6: None,
                })),
                copy_target: None,
                hostname: None,
                split_queues: None,
            }),
            container: None,
            operator: None,
            profile: None,
            sip_binaries: None,
            kube_context: None,
            external_proxy: None,
            internal_proxy: None,
            use_proxy: None,
            experimental: None,
            skip_sip: None,
        };

        assert_eq!(config, expect);
    }

    /// <!--${internal}-->
    /// Helper for printing the config schema.
    ///
    /// Run it with:
    ///
    /// ```sh
    /// cargo test -p mirrord-config print_schema -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn print_schema() {
        let schema = schemars::schema_for!(LayerFileConfig);
        println!("{}", serde_json::to_string_pretty(&schema).unwrap());
    }

    const SCHEMA_FILE_PATH: &str = "../../mirrord-schema.json";

    /// <!--${internal}-->
    /// Writes the config schema to a file (uploaded to the schema store).
    fn write_schema_to_file(schema: &RootSchema) -> File {
        println!("Writing schema to file.");

        let content = serde_json::to_string_pretty(&schema).expect("Failed generating schema!");
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .read(true)
            .open(SCHEMA_FILE_PATH)
            .expect("Failed to create schema file!");

        let _ = file
            .write(content.as_bytes())
            .expect("Failed writing schema to file!");

        file
    }

    /// <!--${internal}-->
    /// Checks if a schema file already exists, otherwise generates the schema and creates the file.
    ///
    /// It also checks and updates when the schema file is outdated.
    ///
    /// Use this function to generate a mirrord config schema file.
    ///
    /// ```sh
    /// cargo test -p mirrord-config check_schema_file_exists_and_is_valid_or_create_it -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn check_schema_file_exists_and_is_valid_or_create_it() {
        let fresh_schema = schemars::schema_for!(LayerFileConfig);
        let fresh_content =
            serde_json::to_string_pretty(&fresh_schema).expect("Failed generating schema!");

        println!("Checking for an existing schema file!");
        let mut existing_content = String::with_capacity(fresh_content.len());
        if File::open(SCHEMA_FILE_PATH)
            .and_then(|mut file| file.read_to_string(&mut existing_content))
            .is_ok()
        {
            if existing_content != fresh_content {
                println!("Schema is outdated, preparing updated version!");
                write_schema_to_file(&fresh_schema);
            }
        } else {
            write_schema_to_file(&fresh_schema);
        }
    }

    #[test]
    fn schema_file_exists() {
        let _ = File::open(SCHEMA_FILE_PATH).expect("Schema file doesn't exist!");
    }

    #[test]
    fn schema_file_is_up_to_date() {
        let compare_schema = schemars::schema_for!(LayerFileConfig);
        let compare_content =
            serde_json::to_string_pretty(&compare_schema).expect("Failed generating schema!");

        let mut existing_content = String::new();
        let _ = File::open(SCHEMA_FILE_PATH)
            .unwrap()
            .read_to_string(&mut existing_content);

        assert_eq!(existing_content.replace("\r\n", "\n"), compare_content);
    }

    /// Related to issue #2936: https://github.com/metalbear-co/mirrord/issues/2936.
    ///
    /// Verifies that [`LayerConfig`] encoded with [`LayerConfig::encode`]
    /// can be decoded back into the same [`LayerConfig`] with [`LayerConfig::decode`].
    #[test]
    fn encode_and_decode_default_config() {
        let mut cfg_context = ConfigContext::default();
        let resolved_config = LayerFileConfig::default()
            .generate_config(&mut cfg_context)
            .expect("Default config should be generated from default 'LayerFileConfig'");

        let encoded = resolved_config.encode().unwrap();
        let decoded = LayerConfig::decode(&encoded).unwrap();

        assert_eq!(decoded, resolved_config);
    }

    /// Same as [`encode_and_decode_default_config`], but uses a more advanced config example.
    #[test]
    fn encode_and_decode_advanced_config() {
        let mut cfg_context = ConfigContext::default();

        // this config includes template variables, so it needs to be rendered first
        let mut template_engine = Tera::default();
        template_engine
            .add_raw_template("main", ADVANCED_CONFIG)
            .unwrap();
        let rendered = template_engine
            .render("main", &tera::Context::new())
            .expect("Tera should render JSON config file contents");
        let resolved_config = ConfigType::Json
            .parse(rendered.as_str())
            .generate_config(&mut cfg_context)
            .expect("Layer config should be generated from JSON config file contents");

        let encoded = resolved_config.encode().unwrap();
        let decoded = LayerConfig::decode(&encoded).unwrap();

        assert_eq!(decoded, resolved_config);
    }

    const ADVANCED_CONFIG: &str = r#"
    {
        "accept_invalid_certificates": false,
        "target": {
            "path": "pod/test-service-abcdefg-abcd",
            "namespace": "default"
        },
        "feature": {
            "env": true,
            "fs": "write",
            "network": {
                "dns": false,
                "incoming": {
                    "mode": "steal",
                    "http_filter": {
                        "header_filter": "x-intercept: {{ get_env(name="USER") }}"
                    }
                },
                "outgoing": {
                    "tcp": true,
                    "udp": false
                }
            }
        }
    }
"#;
}
