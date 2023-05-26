{
  description = "Drill dev";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    geng.url = "github:geng-engine/geng";
  };

  outputs = { nixpkgs, geng, ... }: geng.makeFlakeOutputs (system:
    let pkgs = import nixpkgs {
      inherit system;
    };
    in {
      src = ./.;
      buildInputs = with pkgs; [
      ];
    }
  );
}

