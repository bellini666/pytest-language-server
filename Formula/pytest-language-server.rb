class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.22.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.3/pytest-language-server-aarch64-apple-darwin"
      sha256 "15b9ef0ebd2c486d0ee811e163b64985f982c85e56ca5854ad03558581e07883"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.3/pytest-language-server-x86_64-apple-darwin"
      sha256 "8da9c47aa2f9ee7da7f42bead383cbf1f118fc9cd1047cc8adba639b6642e7a4"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.3/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "65bc8e3d7b52bdd088b6b55c486b920070f35da0bd56a0686c656e31dddf3635"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.22.3/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "1df5d086f8331775a4f0e34bb2fed72e5187141a939c77c60b02bce7fb6771c9"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
