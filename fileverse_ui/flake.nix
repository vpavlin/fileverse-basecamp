{
  description = "Fileverse viewer: pure-QML Basecamp view over fileverse_core";

  inputs = {
    # Same builder rev as fileverse_core and eth_rpc_module 0.1.0.
    logos-module-builder.url = "github:logos-co/logos-module-builder/e1a25706edb50aa7f34d02e08262ae7ffb3cb8db";
    fileverse_core = {
      url = "path:../fileverse_core";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosQmlModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
