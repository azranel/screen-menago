class Smenago < Formula
  desc "Upload screenshots to S3-compatible storage and get a public link"
  homepage "https://github.com/azranel/screen-menago"
  url "https://github.com/azranel/screen-menago/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "fb1146f90fe2399ffe2787810f3f593ee322ceec11ca108a2dfed9f37153512a"
  license "MIT"
  head "https://github.com/azranel/screen-menago.git", branch: "main"

  depends_on "rust" => :build

  # `cargo` downloads crate dependencies from crates.io during the build, and
  # Homebrew otherwise denies network access (and reads of ~/.cargo) in the
  # build sandbox. The test block uses `--dry-run`, so no network is needed there.
  allow_network_access! :build

  def install
    system "cargo", "install", *std_cargo_args
    # `smenago completions <shell>` prints a completion script to stdout.
    generate_completions_from_executable(bin/"smenago", "completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/smenago --version")

    # A dry run computes the object key and public URL without any network
    # call or real credentials, so it is safe to exercise in the sandbox.
    (testpath/"config.json").write <<~JSON
      {
        "account_id": "testaccount",
        "bucket": "test-bucket",
        "access_key_id": "AKIATEST",
        "secret_access_key": "secrettest",
        "public_url_base": "https://pub-test.r2.dev",
        "key_prefix": "screenshots"
      }
    JSON
    (testpath/"shot.png").write("fake-png-bytes")

    output = shell_output(
      "#{bin}/smenago --config #{testpath}/config.json --dry-run --quiet #{testpath}/shot.png",
    )
    assert_match %r{^https://pub-test\.r2\.dev/screenshots/}, output
  end
end
