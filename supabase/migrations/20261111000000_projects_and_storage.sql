-- CampusBIM cloud save: each user's projects (rows) and their .bimcraft files (a private bucket).
-- Applied to the CampusBIM Supabase project; Row Level Security keeps every user to their own rows and files.
create table public.projects (
  id uuid primary key default gen_random_uuid(),
  owner uuid not null default auth.uid() references auth.users(id) on delete cascade,
  name text not null check (char_length(name) between 1 and 200),
  updated_at timestamptz not null default now(),
  size_bytes integer not null default 0 check (size_bytes >= 0),
  storage_path text not null check (storage_path like owner::text || '/%')
);
create index projects_owner_updated_idx on public.projects (owner, updated_at desc);

alter table public.projects enable row level security;

create policy "Owners read their projects" on public.projects for select to authenticated using ((select auth.uid()) = owner);
create policy "Owners add their projects" on public.projects for insert to authenticated with check ((select auth.uid()) = owner);
create policy "Owners change their projects" on public.projects for update to authenticated using ((select auth.uid()) = owner) with check ((select auth.uid()) = owner);
create policy "Owners delete their projects" on public.projects for delete to authenticated using ((select auth.uid()) = owner);

revoke all on public.projects from anon;
grant select, insert, update, delete on public.projects to authenticated;

create or replace function public.projects_touch() returns trigger
language plpgsql set search_path = '' as $$
begin
  new.updated_at := now();
  return new;
end;
$$;
create trigger projects_touch before update on public.projects for each row execute function public.projects_touch();

insert into storage.buckets (id, name, public, file_size_limit)
values ('projects', 'projects', false, 104857600)
on conflict (id) do nothing;

create policy "Users read own project files" on storage.objects for select to authenticated
  using (bucket_id = 'projects' and (storage.foldername(name))[1] = (select auth.uid())::text);
create policy "Users upload own project files" on storage.objects for insert to authenticated
  with check (bucket_id = 'projects' and (storage.foldername(name))[1] = (select auth.uid())::text);
create policy "Users update own project files" on storage.objects for update to authenticated
  using (bucket_id = 'projects' and (storage.foldername(name))[1] = (select auth.uid())::text)
  with check (bucket_id = 'projects' and (storage.foldername(name))[1] = (select auth.uid())::text);
create policy "Users delete own project files" on storage.objects for delete to authenticated
  using (bucket_id = 'projects' and (storage.foldername(name))[1] = (select auth.uid())::text);
