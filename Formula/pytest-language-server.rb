class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.22.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.2/pytest-language-server-aarch64-apple-darwin"
      sha256 "1b134316bfca3425193bc940cf37d5d0e137852027ff0c2b90156fae0fe7743b"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.2/pytest-language-server-x86_64-apple-darwin"
      sha256 "91f983639d14233aaf8c3b647fe0aa332ed77b18202447152b5d2930888118a8"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.2/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "610a30e4fb759982cd83433cbcb0aba1ad8efd21e88bd081afc63dbf8e18f1b3"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.2/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "94bac582fe471a305206ea48822cc05079150002e53a1bc334220c7f36e1d1e7"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
