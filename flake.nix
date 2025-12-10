{
  description = "Search and resume Claude Code conversations";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    naersk.url = "github:nix-community/naersk";
  };

  outputs = {
    self,
    nixpkgs,
    naersk,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    packages = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      naersk' = pkgs.callPackage naersk {};
    in {
      default = naersk'.buildPackage {
        src = ./.;

        meta = with pkgs.lib; {
          description = "Search and resume Claude Code conversations";
          homepage = "https://github.com/zippoxer/recall";
          license = licenses.mit;
          mainProgram = "recall";
        };
      };
    });

    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      default = pkgs.mkShell {
        buildInputs = with pkgs; [cargo rustc rust-analyzer clippy];
      };
    });
  };
}
