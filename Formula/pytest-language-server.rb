class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.24.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.24.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "e9345878a9e06b6ec83636e2ddc4f821269ea4babf67b5c72b14c3b42417964f"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.24.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "23df218d2a9e5e30aecd101517867eb46acc8c66d083d30dba7a96a07fc4e2af"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.24.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "662cc941563094b63c1a88607fe398d878af21e2eac8dd78aaeb2a35332808d2"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.24.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "afff78752cd34e35efcb8b51c54a83c3a2fb4de49f225e5e145241cc48711c30"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
