class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.16.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.1/pytest-language-server-aarch64-apple-darwin"
      sha256 "37f9a2dfdf473e3b5625e735b60769eb7cd848a3a6f2b7dca4580180bdff2eff"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.1/pytest-language-server-x86_64-apple-darwin"
      sha256 "aa83137be26a3d3e43119020225ad70d7cc6aa18edf46f51ad10dea0330ca0dc"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.1/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "e39fb6f73fa7fb53443ef9ffc71352ae9bcc31e74dcd0e149290042eab85623a"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.1/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "368ab3d0547a0797b412e70cb03575cca771c141a2d15f2df320e0a966a3559e"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
