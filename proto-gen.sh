#!/bin/bash -ex
pushd jaeger-idl
mkdir -p proto-gen-python
PROTOC="docker run --rm -u $(id -u) -v${PWD}:${PWD} -w${PWD} jaegertracing/protobuf:0.5.0 --proto_path=${PWD} --experimental_allow_proto3_optional -Iproto/api_v2 -Iproto -I/usr/include/github.com/gogo/protobuf -Iopentelemetry-proto --python_out=proto-gen-python --grpc-python_out=proto-gen-python"
PROTOS=$(docker run --rm -it --entrypoint=/bin/sh jaegertracing/protobuf:0.5.0 -c "find /usr/include -name *.proto")
for proto in $PROTOS
do
    $PROTOC $(echo $proto | tr -d '\r')
done
PROTOS=$(find opentelemetry-proto -name *.proto)
for proto in $PROTOS
do
    $PROTOC $(echo $proto | tr -d '\r')
done
$PROTOC proto/api_v2/model.proto proto/api_v2/query.proto proto/api_v2/collector.proto proto/api_v2/sampling.proto
$PROTOC proto/api_v3/query_service.proto
