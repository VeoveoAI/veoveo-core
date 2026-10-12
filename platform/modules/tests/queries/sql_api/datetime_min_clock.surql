DEFINE FUNCTION fn::kernel::own::at($at: datetime) -> object { RETURN { at: array::min([$at, time::now()]) }; } PERMISSIONS FULL;
