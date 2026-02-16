class PytestLanguageServer < Formula
  desc "Blazingly fast Language Server Protocol implementation for pytest"
  homepage "https://github.com/bellini666/pytest-language-server"
  version "0.19.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.3/pytest-language-server-aarch64-apple-darwin"
      sha256 "71ee92b9666e13c30ac25acf60e64027ef780b00c25d62747617ad8863de6fdc"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.3/pytest-language-server-x86_64-apple-darwin"
      sha256 "440eed44d654a3fc8aa802e711bfc72aa1371c507cb55fde14a0b435a4bf8633"
    end
  end

  on_linux do
    if Hardware::CPU.arm? && Hardware::CPU.is_64_bit?
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.3/pytest-language-server-aarch64-unknown-linux-gnu"
      sha256 "09aae8fb866dd31612ceffe8b276701ad21d057441b8658cb6dd04c0bfab9b87"
    else
      url "https://github.com/bellini666/pytest-language-server/releases/download/v0.19.3/pytest-language-server-x86_64-unknown-linux-gnu"
      sha256 "eb7a389aac1ca679e065784d06d1a7082ce987cca981db65e4e00da9430e6009"
    end
  end

  def install
    bin.install cached_download => "pytest-language-server"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pytest-language-server --version")
  end
end
