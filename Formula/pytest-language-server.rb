class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.25.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.25.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "6bb075eb12c412fd3a5fbbda920001967fe086addbe4d062f6f2af5d610a146d"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.25.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "3ba4cf73e109bad24920a546d40b721f967f6618862f3af4a4a421b6a64ad7df"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.25.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "9cb811913f2155da24391500d5f5276180bc488108bb2c8026231977a67647f1"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.25.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "8720a492d2e2db96d27b57efc103f9df3869d22b259ab9d342e052969392813b"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
