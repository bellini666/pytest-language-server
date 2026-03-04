class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.21.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.2/pytest-language-server-aarch64-apple-darwin"
      sha256 "4b022a52f27ea21448387c15ff2aeec09d75151c60f11c22c9aad7a7fd761af3"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.2/pytest-language-server-x86_64-apple-darwin"
      sha256 "fa897662ff7a58580e76fd5e7883cd9814107f7cfa79e1f45f035b685a2f4c51"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.2/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "2319ddcc0f2ab8b55313fd86839acae7214657895540677f394eb6142e6cb508"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.2/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "78eab332ee205f45abb5bc6c8516a1d2c68a8725b7ea2d3f57258d264bedc5c0"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
