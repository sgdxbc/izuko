import grpc
import sys
from datetime import datetime
from pathlib import Path
from json import dump
from api_v3.query_service_pb2 import *
from api_v3.query_service_pb2_grpc import *
from google.protobuf.timestamp_pb2 import *
from google.protobuf.json_format import MessageToDict

with grpc.insecure_channel(f'{sys.argv[1]}:16685') as channel:
    stub = QueryServiceStub(channel)
    start_time_min = Timestamp()
    start_time_min.FromDatetime(datetime.min)
    start_time_max = Timestamp()
    start_time_max.FromDatetime(datetime.max)
    parameters = TraceQueryParameters(
        service_name='Kubo',
        operation_name='Dual.FindProvidersAsync',
        num_traces=1,
        start_time_min=start_time_min,
        start_time_max=start_time_max,
    )
    trace_spans = []
    trace_id = None
    for traces_data in stub.FindTraces(FindTracesRequest(query=parameters)):
        for resource_spans in traces_data.resource_spans:
            for scope_spans in resource_spans.scope_spans:
                for span in scope_spans.spans:
                    trace_id = trace_id or span.trace_id
                    assert span.trace_id == trace_id
                    trace_spans.append(MessageToDict(span))
                    if span.name == 'Dual.FindProvidersAsync':
                        start_time_min.FromNanoseconds(span.start_time_unix_nano)
                        start_time_max.FromNanoseconds(span.end_time_unix_nano)
                        print(f'* Collect trace of Dual.FindProvidersAsync start {start_time_min.ToJsonString()} end {start_time_max.ToJsonString()}')
    assert trace_id is not None
    print(f'* Total span number {len(trace_spans)}')
    parameters = TraceQueryParameters(
        service_name='Kubo',
        start_time_min=start_time_min,
        start_time_max=start_time_max,
    )
    other_spans = []
    for traces_data in stub.FindTraces(FindTracesRequest(query=parameters)):
        for resource_spans in traces_data.resource_spans:
            for scope_spans in resource_spans.scope_spans:
                for span in scope_spans.spans:
                    if span.trace_id != trace_id:
                        other_spans.append(MessageToDict(span))
    print(f'* Other in time range span number {len(other_spans)}')

path = Path('traces')
path.mkdir(exist_ok=True)
with open(path / f'{start_time_min.ToJsonString()}.json', 'w') as trace_file:
    dump(trace_spans, trace_file)
with open(path / f'{start_time_min.ToJsonString()}_other.json', 'w') as trace_file:
    dump(other_spans, trace_file)
