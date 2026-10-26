{
  description = "fileverse_core: Fileverse dDocs/dSheets for Logos Basecamp, without Fileverse servers";

  inputs = {
    # The builder rev the official EVM modules (eth_rpc_module 0.1.0) ship with.
    logos-module-builder.url = "github:logos-co/logos-module-builder/e1a25706edb50aa7f34d02e08262ae7ffb3cb8db";
    eth_rpc_module = {
      url = "github:logos-co/logos-evm-eth-rpc-module";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
    # Optional: used when installed. Their contracts (LIDL) give the typed clients.
    ipfs_module = {
      url = "github:vpavlin/fileverse-basecamp/c8660fafa98807da394e896f7e5656dcfcb0154c?dir=ipfs_module";
      inputs.logos-module-builder.follows = "logos-module-builder";
    };
    # The storage_module Basecamp 0.3.2 bundles.
    storage_module = {
      url = "github:logos-co/logos-storage-module/v3.0.2";
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
