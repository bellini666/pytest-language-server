class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.15.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.15.0/pytest-language-server-aarch64-apple-darwin"
      sha256 "f8af2496d193bf7527ead42f6fb36675b2f7616faea642f8676b2b98facb0a82"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.15.0/pytest-language-server-x86_64-apple-darwin"
      sha256 "4f689a2c147ec124476880927d2c3df51e455d54c4d5a24187f71627c3de9b4a"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.15.0/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "ec97ae6ea83ac115eb46896ac9e98c6e1a43fc5008e61d8317f0e41fb5b0214e"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.15.0/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "bc03bdc78620687409d49528464b1122d59e29858688aa3c80b7d9c8dcecae21"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
