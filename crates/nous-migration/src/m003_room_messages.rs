use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(RoomMessages::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RoomMessages::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RoomMessages::RoomId).text().not_null())
                    .col(ColumnDef::new(RoomMessages::SenderId).text().not_null())
                    .col(ColumnDef::new(RoomMessages::Content).text().not_null())
                    .col(ColumnDef::new(RoomMessages::ReplyTo).text())
                    .col(ColumnDef::new(RoomMessages::Metadata).text())
                    .col(
                        ColumnDef::new(RoomMessages::CreatedAt)
                            .text()
                            .not_null()
                            .default(Expr::cust("strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RoomMessages::Table, RoomMessages::RoomId)
                            .to(
                                super::m002_rooms::Rooms::Table,
                                super::m002_rooms::Rooms::Id,
                            )
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RoomMessages::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum RoomMessages {
    Table,
    Id,
    RoomId,
    SenderId,
    Content,
    ReplyTo,
    Metadata,
    CreatedAt,
}
