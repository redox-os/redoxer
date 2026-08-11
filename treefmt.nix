_: {
  projectRootFile = "flake.nix";
  programs = {
    dockerfmt = {
      enable = true;
      includes = [
        "Dockerfile"
        "Dockerfile-*"
      ];
    };
    rustfmt.enable = true;
    nixfmt.enable = true;
  };
}
