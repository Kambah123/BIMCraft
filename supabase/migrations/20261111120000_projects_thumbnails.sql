-- Dashboard cards: a picture of each project's view, kept next to its file as <uid>/<id>.png (the bucket's policies
-- already keep each user to their own folder). A rename alone keeps the project's "last edited" date.
alter table public.projects add column thumb_path text check (thumb_path is null or thumb_path like owner::text || '/%');

create or replace function public.projects_touch() returns trigger
language plpgsql set search_path = '' as $$
begin
  if new.name is distinct from old.name
     and (new.size_bytes, new.storage_path, new.thumb_path) is not distinct from (old.size_bytes, old.storage_path, old.thumb_path) then
    new.updated_at := old.updated_at;
  else
    new.updated_at := now();
  end if;
  return new;
end;
$$;
