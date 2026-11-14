class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.26.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.1/pytest-language-server-aarch64-apple-darwin"
      sha256 "88d99556ccae8c37cee6997086b29e11d69fbb2306608cc7e293ff1aa52f8ffb"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.1/pytest-language-server-x86_64-apple-darwin"
      sha256 "a6354bfc96355b13dfc09c7ce792b7a1de488ec37c47c9c2aa74144b5d918e21"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.1/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "f4e3c3adb93b2c8d8bc4f19687915699f3f158f1558b1e8808d7b7f3fafde9f6"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.26.1/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "3ef54124765808896315a1b7be32c79ba279fa0c62bed1ce3dda20c7b6117ece"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
