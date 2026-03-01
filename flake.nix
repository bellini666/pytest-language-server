{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { nixpkgs, ... }:
    let
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs [
          "x86_64-linux"
          "aarch64-linux"
          "x86_64-darwin"
          "aarch64-darwin"
        ] (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "pytest-language-server";
          version = "0.21.1";

          src = pkgs.lib.cleanSource ./.;

          cargoHash = "sha256-cSbJYu6OVfUssNZbKGrixA1+UlOf+5/DIdXjkAKo7cQ=";

          meta = {
            description = "Language Server Protocol implementation for pytest";
            homepage = "https://github.com/bellini666/pytest-language-server";
            license = pkgs.lib.licenses.mit;
            mainProgram = "pytest-language-server";
          };
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
          ];
        };
      });
    };
}
