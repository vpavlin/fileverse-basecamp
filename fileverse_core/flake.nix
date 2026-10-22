{
  description = "fileverse_core: read-only Fileverse dDocs/dSheets viewer core for Logos Basecamp";

  inputs = {
    # The builder rev the official EVM modules (eth_rpc_module 0.1.0) ship with.
    logos-module-builder.url = "github:logos-co/logos-module-builder/e1a25706edb50aa7f34d02e08262ae7ffb3cb8db";
    eth_rpc_module = {
      url = "github:logos-co/logos-evm-eth-rpc-module";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
