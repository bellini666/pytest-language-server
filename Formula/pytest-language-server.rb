class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.21.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.1/pytest-language-server-aarch64-apple-darwin"
      sha256 "17633a91b19c8ce468ccf2df13ff7bbd8272902e003cedb6981710fa0f5fa814"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.1/pytest-language-server-x86_64-apple-darwin"
      sha256 "efe2079253bd09d2179d2dc8986df56c7b731684591f47ac6a8193685f226959"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.1/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "ce1dd4c3a26967a9e10045c251a477c2a7ceeff1bebb70289b9ccf92254fe263"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.21.1/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "26228ebffc8e4d52f811b924dce0db985dd456bf30e4a2ebe71f7e1c11fb670b"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
