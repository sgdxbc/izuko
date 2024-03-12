$ git clone <this repo> --recurse-submodules

Install docker.

$ ./proto-gen.sh
$ pip install protobuf==3.14.0 grpcio

Install kubo on both IPFS peer host and IPFS canary peer host. Install docker on IPFS peer host, ensure sudo-less docker setup. Ensure password-free login to both IPFS peer host and canary peer host.

Initialize IPFS on canary peer host with `ipfs init --profile server`.

Start telemetry daemon on IPFS peer host with
$ docker run -d --rm --name jaeger  -e COLLECTOR_OTLP_ENABLED=true  -e COLLECTOR_ZIPKIN_HOST_PORT=:9411  -p 5775:5775/udp  -p 6831:6831/udp  -p 6832:6832/udp  -p 5778:5778  -p 16686:16686  -p 14250:14250  -p 14268:14268  -p 14269:14269  -p 4317:4317  -p 4318:4318  -p 9411:9411 -p 16685:16685  jaegertracing/all-in-one

Start IPFS daemon on IPFS peer host with
$ OTEL_EXPORTER_OTLP_INSECURE=true OTEL_TRACES_EXPORTER=otlp ipfs daemon --init

Update `ipfs_host` and `ipfs_canary_host` value in `src/main.rs`.

$ cargo run