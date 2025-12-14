class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.16.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "f702d6514c189a644117efbf25efb7115dd2bb0c5494f03ca1d30aa7a80d19bc"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "4e99cccd54aa3c14cd8e300af449a5863899ef70f56a1dee0b4390f96005e111"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "17ac1e52a30b7bf32a169aadb06ab6b3b2d9f9302a081416efa0055c9b7bdab1"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.16.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "1f67766893aed3aeb2b15e83efc08b7c23e8fe8f7a560c7e188fce04120222ad"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
