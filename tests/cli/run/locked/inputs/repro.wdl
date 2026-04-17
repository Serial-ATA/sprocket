version 1.3

task repro {
    command <<<
        cat /etc/lsb-release
    >>>

    requirements {
        # Should be overwritten with ubuntu mantic (much older than latest)
        container: "ubuntu:latest"
    }
}