class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.17.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.1/pytest-language-server-aarch64-apple-darwin"
      sha256 "6f93c93794cdb87b755876103d9ab5340b6a8ece4162242ad3bcf8c12fcdf07d"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.1/pytest-language-server-x86_64-apple-darwin"
      sha256 "9b8917ce555b2085385bde29ed50fa9d61636d96ef86bb5913cbe689b173c901"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.1/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "c98c51ebd9d68086ea5a6bfe6e0a4f3fbf630cba4e74c5c4cc0c0b2d4627baff"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.1/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "7da492dc7bf19eeb6284d3da64bb0fd94f22cd92fd8e6bd0be93fabfd865d53c"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
