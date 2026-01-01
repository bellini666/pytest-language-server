class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.17.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.2/pytest-language-server-aarch64-apple-darwin"
      sha256 "ae2eae342c7da3a14bc23e078d3c2d78d1f8b4609d493924f7549bd5a171e8bf"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.2/pytest-language-server-x86_64-apple-darwin"
      sha256 "6ac4fe83375275c60b174853535de7dc275b0b74353d48e67c84fdcb7aa5f322"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.2/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "523eb39584e6dd2b9d6a42e834ee6c723c79207473c485236447177463f8f7a6"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.17.2/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "42f565bbf9d3cc35c4f69cd9bdff30e8ff48b5d6bef8ae21f839a0a966b59709"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
