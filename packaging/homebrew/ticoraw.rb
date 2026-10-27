# Homebrew formula for ticoraw. It builds from source on the user's machine.
#
# Keep it that way: no `bottle do` block, and do not submit it to homebrew-core (which
# would build and host bottles). Bottles are compiled decoders, and publishing compiled
# decoders is on hold pending patent-licensing review; see packaging/README.md.
#
# scripts/release/update_formula.sh fills in `url` and `sha256` for a release.
class Ticoraw < Formula
  desc "Decoder for Nikon High Efficiency (HE/HE*) NEF raw files"
  homepage "https://github.com/lukaj99/ticoraw"
  url "https://github.com/lukaj99/ticoraw/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/lukaj99/ticoraw.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/ticoraw-cli")
  end

  test do
    assert_equal "ticoraw #{version}", shell_output("#{bin}/ticoraw --version").strip

    (testpath/"not-a-raw.nef").write "not a raw file"
    output = shell_output("#{bin}/ticoraw inspect #{testpath}/not-a-raw.nef 2>&1", 1)
    assert_match "missing TIFF byte-order mark", output
  end
end
