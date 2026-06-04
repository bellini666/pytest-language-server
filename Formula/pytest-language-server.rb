class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.23.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.23.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "d79eab46f7c79c1142648568150a72422e3462116112537cc54330fe38ca7607"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.23.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "765a08590a8ba7296e33faa05a0511a7f3011aeaaab3aca4a7a3bde9123c2a6e"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.23.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "234d2e403ffbce6e6965df531b18295961e2a300edd2d0c91e6a895f12f741f4"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.23.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "af66d52d9b26df51ffb1fb295e44ec792394729f3a604964582b0ba8eaf294bb"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
