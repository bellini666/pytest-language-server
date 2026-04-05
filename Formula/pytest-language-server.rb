class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.22.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "49cfdb39b8c91ccd771b2edace45373bc25a7aecf36a62481af0b2fc05ed0c9e"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "d42dd9bb1c0894cc1a3d7e9374224c4a577433a2675666864fa4796834e4e9a4"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "78590ff87fadf545cb40ef38fda8740a83e39f1b4f16014b66e538f0103faa15"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "d2fb9a5e743b1c40eb592edc94a187f761c4619ed822ee1bb3f07aa374593085"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
