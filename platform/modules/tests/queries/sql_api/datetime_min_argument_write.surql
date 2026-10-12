DEFINE FUNCTION fn::kernel::own::at($at: datetime) -> object { RETURN { at: array::min([{ UPDATE own SET changed = true; RETURN $at; }]) }; } PERMISSIONS FULL;
