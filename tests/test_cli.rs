mod common;

#[cfg(test)]
mod tests {
    use super::common::{create_test_heic, create_test_jpeg, create_test_png};
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::tempdir;

    fn run_info(path: &PathBuf) -> String {
        let output = Command::new("cargo")
            .arg("run")
            .arg("--")
            .arg("--info")
            .arg("--input")
            .arg(path)
            .output()
            .expect("failed to run command");

        if !output.status.success() {
            eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
            eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
            panic!("command failed with status: {}", output.status);
        }
        let stdout = String::from_utf8(output.stdout).unwrap();
        // Print for debugging if needed (optional)
        // eprintln!("stdout:\n{}", stdout);
        stdout
    }

    #[test]
    fn info_shows_details_for_jpeg() {
        let dir = tempdir().unwrap();
        let path = create_test_jpeg(&dir, "test.jpg");
        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100 x 100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Color Space") && stdout.contains("YCbCr"));
        assert!(stdout.contains("Chroma Format")); // value may vary (e.g., 4:2:0, 4:4:4)
    }

    #[test]
    fn info_shows_details_for_png() {
        let dir = tempdir().unwrap();
        let path = create_test_png(&dir, "test.png");
        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100 x 100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Color Space") && stdout.contains("RGB")); // PNG decoded as RGB
        assert!(stdout.contains("Chroma Format") && stdout.contains("4:4:4"));
    }

    #[test]
    fn info_shows_details_for_heic() {
        let dir = tempdir().unwrap();
        let Some(path) = create_test_heic(&dir, "test.heic") else {
            println!("Skipping test: libheif encoding failed.");
            return;
        };
        let stdout = run_info(&path);

        assert!(stdout.contains("Dimensions") && stdout.contains("100 x 100"));
        assert!(stdout.contains("Bit Depth") && stdout.contains("8 bits/channel"));
        assert!(stdout.contains("Alpha Channel") && stdout.contains("No"));
        assert!(stdout.contains("Color Space") && stdout.contains("YCbCr"));
        assert!(stdout.contains("Chroma Format")); // value depends on encoder
    }
}
