version 1.3

task docker_good {
    command <<<>>>

    requirements {
        container: "ubuntu:latest"
    }
}

task docker_good2 {
    command <<<>>>

    requirements {
        container: ["debian:bookworm", "debian:trixie"]
    }
}

task oras_good {
    command <<<>>>

    requirements {
        container: "oras://ghcr.io/stjude-rust-labs/sprocket:v0.23.0"
    }
}

task singularity_good {
    command <<<>>>

    requirements {
        container: "library://ubuntu:latest"
    }
}

task already_locked {
    command <<<>>>

    requirements {
        # Shouldn't show up in the lockfile
        container: "ubuntu@sha256:5e275723f82c67e387ba9e3c24baa0abdcb268917f276a0561c97bef9450d0b4"
    }
}

task already_locked_singularity {
    command <<<>>>

    requirements {
        # Also shouldn't show up in the lockfile
        container: "library://ubuntu:sha256.7a63c14842a5c9b9c0567c1530af87afbb82187444ea45fd7473726ca31a598b"
    }
}

task partial_array {
    command <<<>>>

    requirements {
        # A single unknown image shouldn't invalidate the entire array
        container: ["unknown://foo", "python:3"]
    }
}

task no_container {
    command <<<>>>
}

task placeholders {
    input {
        String python_tag
    }

    command <<<>>>

    requirements {
        container: "python:~{python_tag}"
    }
}
