DEFINE FUNCTION fn::kernel::own::at($at: datetime) -> object { RETURN { at: array::min([$at]) }; } PERMISSIONS FULL;
